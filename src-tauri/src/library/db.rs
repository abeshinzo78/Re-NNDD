//! SQLite ハンドル。
//!
//! プロセス内で 1 本の `Connection` を `tokio::sync::Mutex` で守る。SQLite は
//! 1 接続 1 スレッドの制約があるので、Tauri command が並行で叩いても安全に
//! 直列化される。クエリは数 ms オーダーで終わるため、ここでは pool ではなく
//! 単一接続 + Mutex で十分。WAL モードなので読み取り側は別接続で並行できるが、
//! その必要が出るまでは複雑度を上げない。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use rusqlite::Connection;
use tokio::sync::Mutex;

use crate::error::LibraryError;
use crate::library::schema::run_migrations;

/// DB へのアクセスは全て [`LibraryHandle`] 経由で行う。
pub struct LibraryHandle {
    conn: Mutex<Connection>,
    db_path: PathBuf,
}

impl LibraryHandle {
    /// `path` に SQLite を開いて、未適用の migration を全て流す。
    pub fn open(path: impl AsRef<Path>) -> Result<Arc<Self>, LibraryError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut conn = Connection::open(path)?;
        run_migrations(&mut conn)?;
        Ok(Arc::new(Self {
            conn: Mutex::new(conn),
            db_path: path.to_path_buf(),
        }))
    }

    /// テスト/CLI 用: メモリ DB を開く（migration 済み）。
    #[cfg(test)]
    pub fn open_memory() -> Result<Arc<Self>, LibraryError> {
        let mut conn = Connection::open_in_memory()?;
        run_migrations(&mut conn)?;
        Ok(Arc::new(Self {
            conn: Mutex::new(conn),
            db_path: PathBuf::from(":memory:"),
        }))
    }

    /// クエリ用ロックを取る。
    pub async fn lock(&self) -> tokio::sync::MutexGuard<'_, Connection> {
        self.conn.lock().await
    }

    /// 同期コンテキスト（テスト等）用の blocking lock。
    pub fn blocking_lock(&self) -> tokio::sync::MutexGuard<'_, Connection> {
        self.conn.blocking_lock()
    }

    pub fn db_path(&self) -> &Path {
        &self.db_path
    }
}
