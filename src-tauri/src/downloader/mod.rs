//! Phase 1.2: HLS ダウンローダ。
//!
//! 構成:
//! - [`hls`]: m3u8 パーサ（master / media playlist）
//! - [`fetch`]: Domand 用 niconico-friendly HTTP クライアント
//! - [`run`]: オーケストレータ。1 ジョブ分の DL を実行する

pub mod aes;
pub mod fetch;
pub mod ffmpeg;
pub mod hls;
pub mod mp4box;
pub mod mux;
pub mod run;
pub mod tools;
pub mod ytdlp;
