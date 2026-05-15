//! ダウンロード済み動画ファイルを `<video>` から普通の HTTP で読めるようにする
//! ためのローカル HTTP サーバ。
//!
//! 設計動機: WebKitGTK の `<video>` は Blob URL や `asset://` だと
//! GStreamer のストリームソース扱いになり、後方シーク時に GOP リセットが
//! 雑になって緑ノイズ / 前フレーム残骸が見える ("ガビガビ")。実 HTTP
//! 経由で配信すると WebView の HTTP fetcher が Range リクエストを発行し、
//! GStreamer は普通の HTTP メディアソースとしてシーク可能になる。
//!
//! 実装: `tower-http::services::ServeDir` で Range/206 を含めて静的配信。
//! 127.0.0.1 にだけ bind してランダムポートを使う。
//! 公開エンドポイント: `GET /v/{video_id}/{filename}`

use std::path::PathBuf;

use axum::{
    http::{HeaderValue, Method},
    Router,
};
use tower_http::{cors::CorsLayer, services::ServeDir};

/// `videos_root` 配下を `127.0.0.1:0` (ランダム port) で配信する。
/// 戻り値は実際に bind した port。
///
/// 同期に bind して port を確定させてから、async serve を tokio task に投げる。
/// Tauri の `setup` フック (sync コンテキスト) からそのまま呼べる形にしている。
pub fn start(videos_root: PathBuf) -> std::io::Result<u16> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    listener.set_nonblocking(true)?;
    let port = listener.local_addr()?.port();

    tauri::async_runtime::spawn(async move {
        let listener = match tokio::net::TcpListener::from_std(listener) {
            Ok(l) => l,
            Err(e) => {
                tracing::error!(error = %e, "local_server: TcpListener::from_std failed");
                return;
            }
        };
        // ServeDir は Range / 206 / If-Modified-Since を勝手に扱ってくれる
        let cors = CorsLayer::new()
            .allow_origin(HeaderValue::from_static("*"))
            .allow_methods([Method::GET, Method::HEAD])
            .allow_headers([axum::http::header::RANGE]);
        let app = Router::new()
            .nest_service("/v", ServeDir::new(videos_root))
            .layer(cors);
        tracing::info!(port, "local_server: listening on 127.0.0.1:{port}/v/");
        if let Err(e) = axum::serve(listener, app).await {
            tracing::error!(error = %e, "local_server: serve failed");
        }
    });

    Ok(port)
}

/// 起動済みサーバ情報。Tauri state に登録して frontend のクエリで使う。
#[derive(Debug, Clone)]
pub struct LocalServer {
    pub port: u16,
}
