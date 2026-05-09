# Phase 1.2 段階2: m3u8 パーサのテストリスト

`src-tauri/src/downloader/hls.rs`。

## マスタープレイリスト

- [x] 単一 variant のマスタを `parse_master(text, base)` でパース、絶対 URI が返る
- [x] 複数 variant が `BANDWIDTH desc` 順で返る
- [x] `RESOLUTION=1280x720` を `(1280, 720)` で取る
- [x] `CODECS="avc1.4d401f,mp4a.40.2"` を取る
- [x] 相対 URI (variant.m3u8) は base URL で resolve される
- [x] 絶対 URI (`https://...`) はそのまま通す
- [x] `EXT-X-MEDIA TYPE=AUDIO,GROUP-ID="aac",URI="audio.m3u8"` を audio_groups にパース
- [x] variant の `AUDIO="aac"` から audio_groups[id] を引ける（参照解決）

## メディアプレイリスト

- [x] `EXT-X-MAP:URI="init.cmfv"` を init_uri として返す
- [x] `EXT-X-MAP:URI="init.cmfv",BYTERANGE="200@0"` を init_byte_range として返す
- [x] `EXTINF:6.0,` の次行 URI を Segment にする
- [x] segments が複数あれば全部並ぶ
- [x] `EXT-X-KEY:METHOD=AES-128,URI="key",IV=0x...` を KeyInfo に格納
- [x] `EXT-X-KEY:METHOD=NONE` で暗号化解除
- [x] segment 直前の最新 KEY が segment.key に伝搬する
- [x] `EXT-X-TARGETDURATION:6` を target_duration_sec で読める
- [x] CR/LF と LF どちらの改行でもパース可能
- [x] 行頭 `#` だがディレクティブで無いコメントは無視される

## URI resolve

- [x] base が `https://x.com/a/b/master.m3u8`、URI が `seg/1.cmfv` → `https://x.com/a/b/seg/1.cmfv`
- [x] URI が `/abs/seg.cmfv` → `https://x.com/abs/seg.cmfv`
- [x] URI が `https://other/seg.cmfv` → そのまま
