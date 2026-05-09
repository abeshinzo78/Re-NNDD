//! 1 ジョブ分の HLS ダウンロード実行。
//!
//! API 階層:
//! - [`download_media_playlist`] — media playlist 1 本（映像 or 音声）を
//!   全 segment 並列フェッチして出力ファイルに連結書き出し。AES-128 暗号化
//!   セグメントは fetch 時に復号する。
//! - [`pick_video_and_audio`] — master playlist を読み、最高 bandwidth の
//!   映像 variant + それに紐付く音声 media URL を返す。
//! - [`download_video_track`] — 映像のみ落としたい時の薄いラッパ
//!   （既存ユーザ向け、テストで使用）。
//!
//! 出力は CMAF / fragmented MP4 を `init + seg0 + seg1 ...` の順で連結した
//! バイト列。映像と音声を別ファイルに保存する。単一 MP4 への mux は段階6 以降。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::io::AsyncWriteExt;
use tokio::sync::{Mutex, Semaphore};
use url::Url;

use crate::error::ApiError;

use super::aes::decrypt_aes_128_cbc;
use super::fetch::DomandClient;
use super::hls::{parse_master, parse_media, KeyInfo, MediaPlaylist, Segment};

const DEFAULT_CONCURRENCY: usize = 6;

/// DL 中に発生する事象。`SegmentDone` の累計で進捗 % を出せる。
#[derive(Debug, Clone)]
pub enum Progress {
    /// 解析完了。これから DL する init + segments の合計。
    Started { total_segments: usize },
    /// 1 segment の DL 完了（順序保証なし、index は m3u8 内の順序）。
    SegmentDone { index: usize, bytes: u64 },
    /// 全 segment 完了。最終的に書き出されたファイルサイズ。
    Finished { output_path: PathBuf, total_bytes: u64 },
}

#[derive(Debug, Clone)]
pub struct DownloadOptions {
    pub output_path: PathBuf,
    pub concurrency: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct PickedTracks {
    /// 映像 media playlist URL（必ず 1 本ある）
    pub video_media_url: String,
    /// 音声 media playlist URL（master が EXT-X-MEDIA で持っていれば）
    pub audio_media_url: Option<String>,
    /// 参考: 選択した variant の解像度
    pub resolution: Option<(u32, u32)>,
}

/// master.m3u8 を取得して、最高 bandwidth の video variant と
/// それに紐付く audio group の URL を返す。
pub async fn pick_video_and_audio(
    client: &DomandClient,
    master_url: &str,
) -> Result<PickedTracks, ApiError> {
    let master_url_parsed = Url::parse(master_url)
        .map_err(|e| ApiError::InvalidQuery(format!("invalid master url: {e}")))?;
    let master_text = client.get_text(master_url).await?;
    let master = parse_master(&master_text, &master_url_parsed)?;

    let variant = master.variants.first().ok_or_else(|| {
        ApiError::ResponseShape("master playlist has no video variant".into())
    })?;

    let audio_url = variant
        .audio_group
        .as_ref()
        .and_then(|g| master.audio_groups.get(g))
        .and_then(|alts| {
            alts.iter()
                .find(|a| a.default)
                .or_else(|| alts.first())
                .and_then(|a| a.uri.clone())
        });

    Ok(PickedTracks {
        video_media_url: variant.uri.clone(),
        audio_media_url: audio_url,
        resolution: variant.resolution,
    })
}

/// 1 本の media playlist を全部落として `output_path` に書き出す。
pub async fn download_media_playlist<F>(
    client: &DomandClient,
    media_url: &str,
    options: &DownloadOptions,
    mut on_progress: F,
) -> Result<u64, ApiError>
where
    F: FnMut(Progress) + Send,
{
    let media_url_parsed = Url::parse(media_url)
        .map_err(|e| ApiError::InvalidQuery(format!("invalid media url: {e}")))?;
    let media_text = client.get_text(media_url).await?;
    let media = parse_media(&media_text, &media_url_parsed)?;

    on_progress(Progress::Started {
        total_segments: media.segments.len(),
    });
    let total_bytes = write_concatenated(client, &media, options, &mut on_progress).await?;
    on_progress(Progress::Finished {
        output_path: options.output_path.clone(),
        total_bytes,
    });
    Ok(total_bytes)
}

/// 段階2 互換のラッパ（映像のみ）。
pub async fn download_video_track<F>(
    client: &DomandClient,
    master_url: &str,
    options: &DownloadOptions,
    on_progress: F,
) -> Result<(), ApiError>
where
    F: FnMut(Progress) + Send,
{
    let picked = pick_video_and_audio(client, master_url).await?;
    download_media_playlist(client, &picked.video_media_url, options, on_progress).await?;
    Ok(())
}

async fn write_concatenated<F>(
    client: &DomandClient,
    media: &MediaPlaylist,
    options: &DownloadOptions,
    on_progress: &mut F,
) -> Result<u64, ApiError>
where
    F: FnMut(Progress) + Send,
{
    if let Some(parent) = options.output_path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let tmp_path = with_extension_suffix(&options.output_path, ".part");
    let mut file = tokio::fs::File::create(&tmp_path).await?;
    let mut written: u64 = 0;

    // init segment は最初に 1 回だけ。CMAF init は暗号化されない仕様。
    if let Some(init_uri) = media.init_uri.as_deref() {
        let bytes = match media.init_byte_range {
            Some(br) => {
                client
                    .get_range(init_uri, Some(br.offset), Some(br.offset + br.length - 1))
                    .await?
            }
            None => client.get_bytes(init_uri).await?,
        };
        file.write_all(&bytes).await?;
        written += bytes.len() as u64;
    }

    // 並列 fetch + index 順 write
    let raw = options.concurrency.unwrap_or(0);
    let concurrency = if raw == 0 {
        DEFAULT_CONCURRENCY
    } else {
        raw.clamp(1, 64)
    };
    let sem = Arc::new(Semaphore::new(concurrency));
    let key_cache: Arc<Mutex<HashMap<String, [u8; 16]>>> = Arc::new(Mutex::new(HashMap::new()));

    let mut tasks = Vec::with_capacity(media.segments.len());
    for (idx, seg) in media.segments.iter().enumerate() {
        let permit_sem = Arc::clone(&sem);
        let client_cloned = client.clone();
        let cache = Arc::clone(&key_cache);
        let segment = seg.clone();
        tasks.push(tokio::spawn(async move {
            let _permit = permit_sem
                .acquire_owned()
                .await
                .map_err(|e| ApiError::Downloader(format!("semaphore closed: {e}")))?;
            let bytes = fetch_one_segment(&client_cloned, &segment, idx, &cache).await?;
            Ok::<(usize, Vec<u8>), ApiError>((idx, bytes))
        }));
    }

    let mut buffered: std::collections::BTreeMap<usize, Vec<u8>> = Default::default();
    let mut next_to_write: usize = 0;
    for handle in tasks {
        let (idx, bytes) = handle
            .await
            .map_err(|e| ApiError::Downloader(format!("segment task panicked: {e}")))??;
        let len = bytes.len() as u64;
        on_progress(Progress::SegmentDone { index: idx, bytes: len });
        buffered.insert(idx, bytes);
        while let Some(b) = buffered.remove(&next_to_write) {
            file.write_all(&b).await?;
            written += b.len() as u64;
            next_to_write += 1;
        }
    }
    for (_idx, b) in buffered.into_iter() {
        file.write_all(&b).await?;
        written += b.len() as u64;
    }
    file.flush().await?;
    drop(file);
    tokio::fs::rename(&tmp_path, &options.output_path).await?;
    Ok(written)
}

async fn fetch_one_segment(
    client: &DomandClient,
    segment: &Segment,
    index: usize,
    key_cache: &Arc<Mutex<HashMap<String, [u8; 16]>>>,
) -> Result<Vec<u8>, ApiError> {
    let raw = match segment.byte_range {
        Some(br) => {
            client
                .get_range(&segment.uri, Some(br.offset), Some(br.offset + br.length - 1))
                .await?
        }
        None => client.get_bytes(&segment.uri).await?,
    };
    match &segment.key {
        None => Ok(raw),
        Some(KeyInfo { method, uri, iv }) => {
            if !method.eq_ignore_ascii_case("AES-128") {
                return Err(ApiError::ResponseShape(format!(
                    "unsupported HLS key method: {method}"
                )));
            }
            let key_uri = uri.as_deref().ok_or_else(|| {
                ApiError::ResponseShape("AES-128 key without URI".into())
            })?;
            let key_bytes = fetch_key(client, key_uri, key_cache).await?;
            let iv_bytes = iv.unwrap_or_else(|| super::aes::iv_from_media_sequence(index as u64));
            decrypt_aes_128_cbc(&key_bytes, &iv_bytes, &raw)
        }
    }
}

async fn fetch_key(
    client: &DomandClient,
    key_uri: &str,
    cache: &Arc<Mutex<HashMap<String, [u8; 16]>>>,
) -> Result<[u8; 16], ApiError> {
    // single-flight: 取得中は他のタスクをブロックして二重 fetch を防ぐ。
    // 鍵 fetch は 1 playlist で高々数本なので serialize しても問題なし。
    let mut cache_lock = cache.lock().await;
    if let Some(k) = cache_lock.get(key_uri) {
        return Ok(*k);
    }
    let bytes = client.get_bytes(key_uri).await?;
    if bytes.len() != 16 {
        return Err(ApiError::ResponseShape(format!(
            "AES-128 key {key_uri} must be 16 bytes, got {}",
            bytes.len()
        )));
    }
    let mut k = [0u8; 16];
    k.copy_from_slice(&bytes);
    cache_lock.insert(key_uri.to_string(), k);
    Ok(k)
}

fn with_extension_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut s = path.as_os_str().to_os_string();
    s.push(suffix);
    PathBuf::from(s)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::api::auth::SessionStore;
    use aes::cipher::{block_padding::Pkcs7, BlockEncryptMut, KeyIvInit};
    use std::sync::Arc;

    type Aes128CbcEnc = cbc::Encryptor<aes::Aes128>;

    fn aes_encrypt(key: &[u8; 16], iv: &[u8; 16], plaintext: &[u8]) -> Vec<u8> {
        let mut buf = vec![0u8; plaintext.len() + 16];
        let n = Aes128CbcEnc::new(key.into(), iv.into())
            .encrypt_padded_b2b_mut::<Pkcs7>(plaintext, &mut buf)
            .unwrap()
            .len();
        buf.truncate(n);
        buf
    }

    #[tokio::test]
    async fn end_to_end_video_only_download() {
        let mut server = mockito::Server::new_async().await;
        let base = server.url();

        let init_bytes = b"INIT_BYTES____AAAA";
        let seg0 = vec![0xAAu8; 32];
        let seg1 = vec![0xBBu8; 64];
        let seg2 = vec![0xCCu8; 16];

        let _m_master = server
            .mock("GET", "/v1/abc/master.m3u8")
            .with_status(200)
            .with_header("content-type", "application/vnd.apple.mpegurl")
            .with_body(
                "#EXTM3U\n\
                 #EXT-X-STREAM-INF:BANDWIDTH=1500000,RESOLUTION=1280x720\n\
                 720p/playlist.m3u8\n\
                 #EXT-X-STREAM-INF:BANDWIDTH=500000,RESOLUTION=640x360\n\
                 360p/playlist.m3u8\n",
            )
            .create_async()
            .await;
        let _m_media = server
            .mock("GET", "/v1/abc/720p/playlist.m3u8")
            .with_status(200)
            .with_body(
                "#EXTM3U\n\
                 #EXT-X-VERSION:7\n\
                 #EXT-X-TARGETDURATION:6\n\
                 #EXT-X-MAP:URI=\"init.cmfv\"\n\
                 #EXTINF:6.0,\n\
                 seg0.cmfv\n\
                 #EXTINF:6.0,\n\
                 seg1.cmfv\n\
                 #EXTINF:6.0,\n\
                 seg2.cmfv\n\
                 #EXT-X-ENDLIST\n",
            )
            .create_async()
            .await;
        let _m_init = server
            .mock("GET", "/v1/abc/720p/init.cmfv")
            .with_status(200)
            .with_body(init_bytes)
            .create_async()
            .await;
        let _m_s0 = server
            .mock("GET", "/v1/abc/720p/seg0.cmfv")
            .with_status(200)
            .with_body(seg0.clone())
            .create_async()
            .await;
        let _m_s1 = server
            .mock("GET", "/v1/abc/720p/seg1.cmfv")
            .with_status(200)
            .with_body(seg1.clone())
            .create_async()
            .await;
        let _m_s2 = server
            .mock("GET", "/v1/abc/720p/seg2.cmfv")
            .with_status(200)
            .with_body(seg2.clone())
            .create_async()
            .await;

        let session = Arc::new(SessionStore::default());
        let client = DomandClient::new(session).unwrap();
        let tmpdir = tempfile::tempdir().unwrap();
        let out_path = tmpdir.path().join("video.mp4");
        let opts = DownloadOptions {
            output_path: out_path.clone(),
            concurrency: Some(2),
        };
        let mut events: Vec<Progress> = Vec::new();
        download_video_track(&client, &format!("{base}/v1/abc/master.m3u8"), &opts, |p| {
            events.push(p)
        })
        .await
        .unwrap();

        let written = std::fs::read(&out_path).unwrap();
        let mut expected = Vec::new();
        expected.extend_from_slice(init_bytes);
        expected.extend_from_slice(&seg0);
        expected.extend_from_slice(&seg1);
        expected.extend_from_slice(&seg2);
        assert_eq!(written, expected);
        assert!(matches!(events.first().unwrap(), Progress::Started { total_segments: 3 }));
        assert!(matches!(events.last().unwrap(), Progress::Finished { .. }));
    }

    #[tokio::test]
    async fn aes_encrypted_segments_decrypt_correctly() {
        let mut server = mockito::Server::new_async().await;
        let base = server.url();

        let key = [0x77u8; 16];
        let iv: [u8; 16] = [
            0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D,
            0x0E, 0x0F,
        ];

        let init_plain = b"INIT_DATA_FRAGMENT".to_vec();
        let seg0_plain = vec![0x11u8; 1000];
        let seg1_plain = vec![0x22u8; 2000];
        let seg0_cipher = aes_encrypt(&key, &iv, &seg0_plain);
        let seg1_cipher = aes_encrypt(&key, &iv, &seg1_plain);

        let iv_hex = "0x000102030405060708090A0B0C0D0E0F";
        let _m_master = server
            .mock("GET", "/master.m3u8")
            .with_status(200)
            .with_body(
                "#EXTM3U\n\
                 #EXT-X-STREAM-INF:BANDWIDTH=1500000,RESOLUTION=1280x720\n\
                 v.m3u8\n",
            )
            .create_async()
            .await;
        let _m_media = server
            .mock("GET", "/v.m3u8")
            .with_status(200)
            .with_body(format!(
                "#EXTM3U\n\
                 #EXT-X-MAP:URI=\"i.cmfv\"\n\
                 #EXT-X-KEY:METHOD=AES-128,URI=\"k.bin\",IV={iv_hex}\n\
                 #EXTINF:6.0,\n\
                 s0.cmfv\n\
                 #EXTINF:6.0,\n\
                 s1.cmfv\n\
                 #EXT-X-ENDLIST\n"
            ))
            .create_async()
            .await;
        let _m_key = server
            .mock("GET", "/k.bin")
            .with_status(200)
            .with_body(key)
            .expect_at_most(1) // 鍵キャッシュが効いていることを確認
            .create_async()
            .await;
        let _m_init = server
            .mock("GET", "/i.cmfv")
            .with_status(200)
            .with_body(init_plain.clone())
            .create_async()
            .await;
        let _m_s0 = server
            .mock("GET", "/s0.cmfv")
            .with_status(200)
            .with_body(seg0_cipher)
            .create_async()
            .await;
        let _m_s1 = server
            .mock("GET", "/s1.cmfv")
            .with_status(200)
            .with_body(seg1_cipher)
            .create_async()
            .await;

        let session = Arc::new(SessionStore::default());
        let client = DomandClient::new(session).unwrap();
        let tmpdir = tempfile::tempdir().unwrap();
        let out_path = tmpdir.path().join("video.mp4");
        let opts = DownloadOptions {
            output_path: out_path.clone(),
            concurrency: Some(4),
        };
        download_video_track(&client, &format!("{base}/master.m3u8"), &opts, |_| {})
            .await
            .unwrap();

        let written = std::fs::read(&out_path).unwrap();
        let mut expected = Vec::new();
        expected.extend_from_slice(&init_plain);
        expected.extend_from_slice(&seg0_plain);
        expected.extend_from_slice(&seg1_plain);
        assert_eq!(written, expected);
        _m_key.assert_async().await;
    }

    #[tokio::test]
    async fn pick_video_and_audio_resolves_audio_group() {
        let mut server = mockito::Server::new_async().await;
        let base = server.url();

        let _m_master = server
            .mock("GET", "/master.m3u8")
            .with_status(200)
            .with_body(
                "#EXTM3U\n\
                 #EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"aac\",NAME=\"main\",DEFAULT=YES,URI=\"audio.m3u8\"\n\
                 #EXT-X-STREAM-INF:BANDWIDTH=3000000,RESOLUTION=1920x1080,AUDIO=\"aac\"\n\
                 hi/v.m3u8\n\
                 #EXT-X-STREAM-INF:BANDWIDTH=500000,RESOLUTION=640x360,AUDIO=\"aac\"\n\
                 lo/v.m3u8\n",
            )
            .create_async()
            .await;

        let session = Arc::new(SessionStore::default());
        let client = DomandClient::new(session).unwrap();
        let picked = pick_video_and_audio(&client, &format!("{base}/master.m3u8"))
            .await
            .unwrap();
        assert!(picked.video_media_url.ends_with("/hi/v.m3u8"));
        assert!(picked.audio_media_url.as_deref().unwrap().ends_with("/audio.m3u8"));
        assert_eq!(picked.resolution, Some((1920, 1080)));
    }
}
