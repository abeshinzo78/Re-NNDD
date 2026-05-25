//! ZIP からプラグインを取り出して `$APPDATA/plugins/<id>/` に展開する。
//!
//! 安全性の柱:
//! - `enclosed_name()` (zip 0.6) で `..` / 絶対パスを 1 段目で拒否
//! - 念のため手動でも `..` / `/` 先頭 / Windows ドライブ文字 (`:`) を弾く
//! - 展開は tmp ディレクトリに行い、最後に rename — 失敗時に半端な
//!   `plugins/<id>/` が残らない
//! - 既存 ID と衝突したら `replace=false` のときは `AlreadyInstalled` を返す
//!
//! テスト容易性のため `install_from_zip_bytes` (bytes ベース) を分離し、
//! ファイルパス版はその薄いラッパとした。

use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use parking_lot::Mutex;

use crate::plugins::manifest::{ManifestError, PluginManifest};

/// プロセス内で衝突しない tmp/backup ディレクトリ名を作るためのカウンタ。
/// 同一プラグイン ID を同時に並列インストールしようとしたとき、片方の
/// `remove_dir_all` がもう片方の展開中ディレクトリを巻き込まないよう、
/// pid だけでなくこの値も name に含める (Codex review r3297535062)。
static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

fn unique_suffix() -> String {
    let n = TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{}-{n}", std::process::id())
}

/// インストール処理全体を直列化するグローバルロック。同一プロセス内で
/// 並列に `install_from_zip_*` が走ると、片方の backup → rename → restore
/// シーケンスがもう片方の作業を巻き戻して既に成功した新版を上書き
/// する race がある (Codex review r3297638379)。ユーザ操作起点のため
/// 性能上問題にならない範囲で全インストールを直列化する。
static INSTALL_MUTEX: Mutex<()> = Mutex::new(());

#[derive(Debug, thiserror::Error)]
pub enum InstallError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("zip error: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("manifest error: {0}")]
    Manifest(#[from] ManifestError),
    #[error("manifest.json not found in zip")]
    ManifestMissing,
    #[error("plugin already installed: {0}")]
    AlreadyInstalled(String),
    #[error("unsafe path in zip: {0}")]
    UnsafePath(String),
    #[error("declared entry {entry:?} not present in zip")]
    EntryMissing { entry: String },
    #[error("size limit exceeded ({reason})")]
    SizeLimit { reason: String },
}

// 解凍時のサイズガード (Codex review r3297741207: zip-bomb DoS 防止)。
// MVP プラグインの想定 (manifest + index.js + 軽量 assets) を大きく超える
// 値を上限とする。1 エントリ 50 MiB / 合計 200 MiB。
const MAX_ENTRY_BYTES: u64 = 50 * 1024 * 1024;
const MAX_TOTAL_BYTES: u64 = 200 * 1024 * 1024;

#[derive(Debug)]
pub struct InstallResult {
    pub manifest: PluginManifest,
    pub installed_at: PathBuf,
}

pub async fn install_from_zip_path(
    plugins_root: &Path,
    zip_path: &Path,
    replace: bool,
    app_version: &str,
) -> Result<InstallResult, InstallError> {
    let bytes = tokio::fs::read(zip_path).await?;
    install_from_zip_bytes(plugins_root, &bytes, replace, app_version)
}

pub fn install_from_zip_bytes(
    plugins_root: &Path,
    zip_bytes: &[u8],
    replace: bool,
    app_version: &str,
) -> Result<InstallResult, InstallError> {
    // インストール全体を直列化する (concurrent replace で backup/rename が
    // 互いを巻き戻すレースを防ぐ — Codex review r3297638379)。
    let _guard = INSTALL_MUTEX.lock();
    let cursor = Cursor::new(zip_bytes);
    let mut archive = zip::ZipArchive::new(cursor)?;

    let manifest_text = read_manifest_text(&mut archive)?;
    let manifest = PluginManifest::parse_and_validate(&manifest_text, Some(app_version))?;

    // entry がアーカイブに存在することを確認。ディレクトリ entry は
    // 動的 import の対象にならないので非ディレクトリのみを受け付ける
    // (Codex review r3297535041)。
    let mut has_entry = false;
    for i in 0..archive.len() {
        let file = archive.by_index(i)?;
        let safe = safe_relative_path(&file)?;
        if !file.is_dir() && safe.to_string_lossy() == manifest.entry {
            has_entry = true;
            break;
        }
    }
    if !has_entry {
        return Err(InstallError::EntryMissing {
            entry: manifest.entry.clone(),
        });
    }

    let target = plugins_root.join(&manifest.id);
    if target.exists() && !replace {
        return Err(InstallError::AlreadyInstalled(manifest.id.clone()));
    }

    // ① まず tmp に全部展開する (target には触らない)。展開途中で zip 破損
    //   や IO エラーが出ても、既存のプラグインは無傷で残る (Codex review #3)。
    //   tmp / backup には pid + プロセス内カウンタ由来の unique サフィックス
    //   を付け、同 plugin id への並列 install が互いの作業領域を踏まないようにする。
    let suffix = unique_suffix();
    let tmp = plugins_root.join(format!("{}.tmp-{}", manifest.id, suffix));
    if tmp.exists() {
        std::fs::remove_dir_all(&tmp)?;
    }
    std::fs::create_dir_all(&tmp)?;

    let extract_result: Result<(), InstallError> = (|| {
        let mut total_bytes: u64 = 0;
        for i in 0..archive.len() {
            let mut file = archive.by_index(i)?;
            let safe = safe_relative_path(&file)?;
            let out_path = tmp.join(&safe);
            if file.is_dir() {
                std::fs::create_dir_all(&out_path)?;
                continue;
            }
            if let Some(parent) = out_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut out = std::fs::File::create(&out_path)?;
            // 各エントリの展開を MAX_ENTRY_BYTES + 1 で打ち切り、超過したら拒否
            // する。zip の "圧縮率は高いが原寸は巨大" な zip-bomb を防ぐ。
            // 合計サイズも MAX_TOTAL_BYTES で打ち切る。
            let mut limited = std::io::Read::take(file.by_ref(), MAX_ENTRY_BYTES + 1);
            let copied = std::io::copy(&mut limited, &mut out)?;
            if copied > MAX_ENTRY_BYTES {
                return Err(InstallError::SizeLimit {
                    reason: format!(
                        "entry {} exceeded {} bytes",
                        safe.display(),
                        MAX_ENTRY_BYTES
                    ),
                });
            }
            total_bytes = total_bytes.saturating_add(copied);
            if total_bytes > MAX_TOTAL_BYTES {
                return Err(InstallError::SizeLimit {
                    reason: format!("total extracted size exceeded {} bytes", MAX_TOTAL_BYTES),
                });
            }
        }
        Ok(())
    })();

    if let Err(e) = extract_result {
        let _ = std::fs::remove_dir_all(&tmp);
        return Err(e);
    }

    // ② 展開成功。target が既存ならいったん backup へ退避してから tmp を
    //   target に rename する。final rename 失敗時は backup を target に
    //   戻して "なかったこと" にする。
    let backup = if target.exists() {
        let bk = plugins_root.join(format!("{}.bak-{}", manifest.id, suffix));
        if bk.exists() {
            std::fs::remove_dir_all(&bk)?;
        }
        if let Err(e) = std::fs::rename(&target, &bk) {
            let _ = std::fs::remove_dir_all(&tmp);
            return Err(InstallError::Io(e));
        }
        Some(bk)
    } else {
        None
    };

    if let Err(e) = std::fs::rename(&tmp, &target) {
        let _ = std::fs::remove_dir_all(&tmp);
        if let Some(bk) = backup {
            // 旧プラグインを元に戻す。これ自体が失敗するとユーザ介入が必要
            // だが、backup ディレクトリ自体は残しておくのでデータ消失は無い。
            let _ = std::fs::rename(&bk, &target);
        }
        return Err(InstallError::Io(e));
    }

    // 成功 → backup を片付ける (失敗しても致命ではない、次回再 install で
    // 旧 backup ディレクトリは tmp 衝突防止と同様に削除される)。
    if let Some(bk) = backup {
        let _ = std::fs::remove_dir_all(&bk);
    }

    Ok(InstallResult {
        manifest,
        installed_at: target,
    })
}

pub fn uninstall(plugins_root: &Path, plugin_id: &str) -> Result<(), InstallError> {
    if !crate::plugins::manifest::is_valid_plugin_id(plugin_id) {
        return Err(InstallError::UnsafePath(plugin_id.to_string()));
    }
    let target = plugins_root.join(plugin_id);
    if target.exists() {
        std::fs::remove_dir_all(&target)?;
    }
    Ok(())
}

fn read_manifest_text<R: Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
) -> Result<String, InstallError> {
    let mut file = match archive.by_name("manifest.json") {
        Ok(f) => f,
        Err(zip::result::ZipError::FileNotFound) => return Err(InstallError::ManifestMissing),
        Err(e) => return Err(InstallError::Zip(e)),
    };
    let mut s = String::new();
    file.read_to_string(&mut s)?;
    Ok(s)
}

fn safe_relative_path(file: &zip::read::ZipFile<'_>) -> Result<PathBuf, InstallError> {
    let raw = file.name().to_string();
    if raw.is_empty() {
        return Err(InstallError::UnsafePath(raw));
    }
    // zip 0.6 の enclosed_name() は `..` や絶対パスを排除する
    let enclosed = file
        .enclosed_name()
        .ok_or_else(|| InstallError::UnsafePath(raw.clone()))?;
    // 念のため明示的な弾き直しも行う
    if raw.contains("..") || raw.starts_with('/') || raw.starts_with('\\') || raw.contains(':') {
        return Err(InstallError::UnsafePath(raw));
    }
    Ok(enclosed.to_path_buf())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;
    use zip::write::FileOptions;

    fn zip_with(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut buf = Vec::new();
        {
            let mut writer = zip::ZipWriter::new(Cursor::new(&mut buf));
            let opts = FileOptions::default().compression_method(zip::CompressionMethod::Deflated);
            for (name, body) in files {
                writer.start_file(*name, opts).unwrap();
                writer.write_all(body).unwrap();
            }
            writer.finish().unwrap();
        }
        buf
    }

    fn manifest_bytes(id: &str) -> Vec<u8> {
        serde_json::json!({
            "id": id,
            "name": "Demo",
            "version": "0.1.0",
            "entry": "index.js"
        })
        .to_string()
        .into_bytes()
    }

    #[test]
    fn happy_path_extracts() {
        let tmp = TempDir::new().unwrap();
        let body = zip_with(&[
            ("manifest.json", &manifest_bytes("com.example.ok")),
            ("index.js", b"export const x = 1;"),
        ]);
        let res = install_from_zip_bytes(tmp.path(), &body, false, "0.1.0").unwrap();
        assert_eq!(res.manifest.id, "com.example.ok");
        let entry = res.installed_at.join("index.js");
        assert!(entry.exists());
    }

    #[test]
    fn missing_manifest_rejected() {
        let tmp = TempDir::new().unwrap();
        let body = zip_with(&[("index.js", b"")]);
        let err = install_from_zip_bytes(tmp.path(), &body, false, "0.1.0").unwrap_err();
        matches!(err, InstallError::ManifestMissing);
    }

    #[test]
    fn missing_entry_rejected() {
        let tmp = TempDir::new().unwrap();
        let body = zip_with(&[("manifest.json", &manifest_bytes("com.example.x"))]);
        let err = install_from_zip_bytes(tmp.path(), &body, false, "0.1.0").unwrap_err();
        matches!(err, InstallError::EntryMissing { .. });
    }

    #[test]
    fn directory_entry_does_not_satisfy_entry_field() {
        // manifest.entry が "index.js" でも、アーカイブに "index.js" という
        // ディレクトリしか入っていない場合は EntryMissing で拒否する。動的
        // import の対象になる実体ファイルが要る (Codex review r3297535041)。
        let tmp = TempDir::new().unwrap();
        let mut buf = Vec::new();
        {
            let mut writer = zip::ZipWriter::new(Cursor::new(&mut buf));
            let opts = zip::write::FileOptions::default();
            writer.start_file("manifest.json", opts).unwrap();
            std::io::Write::write_all(&mut writer, &manifest_bytes("com.example.dir")).unwrap();
            // index.js を **ディレクトリ** として登録
            writer.add_directory("index.js", opts).unwrap();
            writer.finish().unwrap();
        }
        let err = install_from_zip_bytes(tmp.path(), &buf, false, "0.1.0").unwrap_err();
        assert!(
            matches!(err, InstallError::EntryMissing { .. }),
            "expected EntryMissing, got {err:?}"
        );
    }

    #[test]
    fn duplicate_install_without_replace_rejected() {
        let tmp = TempDir::new().unwrap();
        let body = zip_with(&[
            ("manifest.json", &manifest_bytes("com.example.dup")),
            ("index.js", b""),
        ]);
        install_from_zip_bytes(tmp.path(), &body, false, "0.1.0").unwrap();
        let err = install_from_zip_bytes(tmp.path(), &body, false, "0.1.0").unwrap_err();
        matches!(err, InstallError::AlreadyInstalled(_));
    }

    #[test]
    fn duplicate_with_replace_succeeds() {
        let tmp = TempDir::new().unwrap();
        let body = zip_with(&[
            ("manifest.json", &manifest_bytes("com.example.dup2")),
            ("index.js", b""),
        ]);
        install_from_zip_bytes(tmp.path(), &body, false, "0.1.0").unwrap();
        install_from_zip_bytes(tmp.path(), &body, true, "0.1.0").unwrap();
    }

    #[test]
    fn replace_swaps_contents_to_new_version() {
        // v1 → v2 で index.js の中身が確実に置き換わることを検証 (backup + rename
        // 経由の atomic swap が機能している)。
        let tmp = TempDir::new().unwrap();
        let v1 = zip_with(&[
            ("manifest.json", &manifest_bytes("com.example.swap")),
            ("index.js", b"export const v = 1;"),
        ]);
        let v2 = zip_with(&[
            ("manifest.json", &manifest_bytes("com.example.swap")),
            ("index.js", b"export const v = 2;"),
        ]);
        let r1 = install_from_zip_bytes(tmp.path(), &v1, false, "0.1.0").unwrap();
        let written_v1 = std::fs::read_to_string(r1.installed_at.join("index.js")).unwrap();
        assert_eq!(written_v1, "export const v = 1;");
        let r2 = install_from_zip_bytes(tmp.path(), &v2, true, "0.1.0").unwrap();
        let written_v2 = std::fs::read_to_string(r2.installed_at.join("index.js")).unwrap();
        assert_eq!(written_v2, "export const v = 2;");
        // backup ディレクトリは片付いている
        let leftovers: Vec<_> = std::fs::read_dir(tmp.path())
            .unwrap()
            .filter_map(|e| e.ok().map(|e| e.file_name().to_string_lossy().into_owned()))
            .filter(|n| n.contains(".bak-") || n.contains(".tmp-"))
            .collect();
        assert!(leftovers.is_empty(), "leftover dirs: {leftovers:?}");
    }

    #[test]
    fn uninstall_removes_dir() {
        let tmp = TempDir::new().unwrap();
        let body = zip_with(&[
            ("manifest.json", &manifest_bytes("com.example.rm")),
            ("index.js", b""),
        ]);
        let res = install_from_zip_bytes(tmp.path(), &body, false, "0.1.0").unwrap();
        assert!(res.installed_at.exists());
        uninstall(tmp.path(), "com.example.rm").unwrap();
        assert!(!res.installed_at.exists());
    }

    #[test]
    fn uninstall_invalid_id_rejected() {
        let tmp = TempDir::new().unwrap();
        let err = uninstall(tmp.path(), "../etc").unwrap_err();
        matches!(err, InstallError::UnsafePath(_));
    }

    #[test]
    fn per_entry_size_limit_rejects_oversized_entry() {
        // zip-bomb 風: 1 エントリが MAX_ENTRY_BYTES (50 MiB) を超えるケース。
        // ここでは Stored (無圧縮) で大きいデータを入れて確認する
        // (Codex review r3297741207)。
        let tmp = TempDir::new().unwrap();
        let mut buf = Vec::new();
        {
            let mut writer = zip::ZipWriter::new(Cursor::new(&mut buf));
            let opts = zip::write::FileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            writer.start_file("manifest.json", opts).unwrap();
            std::io::Write::write_all(&mut writer, &manifest_bytes("com.example.bomb")).unwrap();
            writer.start_file("index.js", opts).unwrap();
            std::io::Write::write_all(&mut writer, b"export const x = 1;").unwrap();
            // 大きい無圧縮エントリ (アサート可能なサイズに留めるが MAX_ENTRY_BYTES 超え)
            writer.start_file("big.bin", opts).unwrap();
            // 50 MiB + 1 byte 書き込む
            let chunk = vec![0u8; 1024 * 1024]; // 1 MiB
            for _ in 0..50 {
                std::io::Write::write_all(&mut writer, &chunk).unwrap();
            }
            std::io::Write::write_all(&mut writer, b"X").unwrap();
            writer.finish().unwrap();
        }
        let err = install_from_zip_bytes(tmp.path(), &buf, false, "0.1.0").unwrap_err();
        assert!(
            matches!(err, InstallError::SizeLimit { .. }),
            "expected SizeLimit, got {err:?}"
        );
        // 失敗時の cleanup: 部分展開 tmp が残らない
        let leftovers: Vec<_> = std::fs::read_dir(tmp.path())
            .unwrap()
            .filter_map(|e| e.ok().map(|e| e.file_name().to_string_lossy().into_owned()))
            .filter(|n| n.contains(".tmp-") || n == "com.example.bomb")
            .collect();
        assert!(
            leftovers.is_empty(),
            "leftover dirs after rejection: {leftovers:?}"
        );
    }
}
