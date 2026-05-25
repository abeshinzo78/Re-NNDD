//! プロセス内のプラグイン状態。DB が canonical だが、dispatcher が
//! 1 リクエストあたり何度も SQL を叩かなくて済むよう、有効/無効と
//! manifest を in-memory にキャッシュする。
//!
//! 真値は plugins テーブル。`reload_from_db` で同期する。

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::RwLock;
use rusqlite::Connection;

use crate::library::db::LibraryHandle;
use crate::plugins::manifest::PluginManifest;
use crate::plugins::registry;

#[derive(Debug, Clone)]
pub struct PluginEntry {
    pub manifest: PluginManifest,
    pub enabled: bool,
}

#[derive(Default)]
pub struct PluginRuntime {
    inner: RwLock<HashMap<String, PluginEntry>>,
}

impl PluginRuntime {
    pub fn get(&self, id: &str) -> Option<PluginEntry> {
        self.inner.read().get(id).cloned()
    }

    pub fn list(&self) -> Vec<PluginEntry> {
        let mut v: Vec<PluginEntry> = self.inner.read().values().cloned().collect();
        v.sort_by(|a, b| a.manifest.id.cmp(&b.manifest.id));
        v
    }

    pub fn upsert(&self, id: &str, manifest: PluginManifest, enabled: bool) {
        self.inner
            .write()
            .insert(id.to_string(), PluginEntry { manifest, enabled });
    }

    /// `id` が runtime cache に存在しないと silent no-op になり DB と乖離する
    /// (Codex #11) ため、true/false を返して呼出側に判定させる。
    /// 戻値 true = 反映済み、false = cache miss (DB に存在しても runtime に
    /// 入っていない壊れた manifest 等)。
    pub fn set_enabled(&self, id: &str, enabled: bool) -> bool {
        if let Some(entry) = self.inner.write().get_mut(id) {
            entry.enabled = enabled;
            true
        } else {
            false
        }
    }

    pub fn remove(&self, id: &str) {
        self.inner.write().remove(id);
    }

    /// DB 全件読み込みで in-memory を再構築する。起動時 + install/uninstall
    /// 完了時に呼ぶ。manifest_json の parse は best-effort で、壊れた行は
    /// 警告ログを出して **skip** する (1 つの壊れた行で全プラグインが
    /// 失われないようにする)。
    pub fn reload_from_db(&self, conn: &Connection) -> Result<(), rusqlite::Error> {
        let rows = registry::list_all(conn).map_err(|e| match e {
            crate::error::LibraryError::Sqlite(e) => e,
            other => rusqlite::Error::ToSqlConversionFailure(Box::new(std::io::Error::other(
                other.to_string(),
            ))),
        })?;
        let mut next: HashMap<String, PluginEntry> = HashMap::new();
        for row in rows {
            match PluginManifest::parse_and_validate(&row.manifest_json, None) {
                Ok(m) => {
                    next.insert(
                        row.plugin_id.clone(),
                        PluginEntry {
                            manifest: m,
                            enabled: row.enabled,
                        },
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        plugin_id = %row.plugin_id,
                        error = %e,
                        "skipping plugin with invalid manifest_json"
                    );
                }
            }
        }
        *self.inner.write() = next;
        Ok(())
    }
}

/// 起動時に呼ぶヘルパ。LibraryHandle から接続を取って `reload_from_db`。
pub async fn bootstrap(runtime: &Arc<PluginRuntime>, library: &Arc<LibraryHandle>) {
    let conn = library.lock().await;
    if let Err(e) = runtime.reload_from_db(&conn) {
        tracing::error!(error = %e, "plugin runtime initial load failed");
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn mk_manifest(id: &str) -> PluginManifest {
        PluginManifest {
            id: id.into(),
            name: "T".into(),
            version: "0.1.0".into(),
            entry: "index.js".into(),
            description: None,
            author: None,
            homepage: None,
            min_app_version: None,
            permissions: vec![],
        }
    }

    #[test]
    fn upsert_get_remove() {
        let r = PluginRuntime::default();
        assert!(r.get("a").is_none());
        r.upsert("a", mk_manifest("a"), true);
        let got = r.get("a").unwrap();
        assert!(got.enabled);
        assert_eq!(got.manifest.id, "a");
        assert!(r.set_enabled("a", false));
        assert!(!r.get("a").unwrap().enabled);
        r.remove("a");
        assert!(r.get("a").is_none());
    }

    #[test]
    fn set_enabled_returns_false_on_cache_miss() {
        let r = PluginRuntime::default();
        assert!(!r.set_enabled("missing", true));
    }

    #[test]
    fn list_is_sorted() {
        let r = PluginRuntime::default();
        r.upsert("b", mk_manifest("b"), true);
        r.upsert("a", mk_manifest("a"), false);
        let v = r.list();
        assert_eq!(v[0].manifest.id, "a");
        assert_eq!(v[1].manifest.id, "b");
    }

    #[test]
    fn reload_from_db_skips_invalid_rows() {
        let lib = LibraryHandle::open_memory().unwrap();
        let conn = lib.blocking_lock();
        registry::upsert(&conn, "good", "0.1.0", &valid_manifest_json("good"), 1).unwrap();
        registry::upsert(&conn, "bad", "0.1.0", "{not json", 2).unwrap();
        let runtime = PluginRuntime::default();
        runtime.reload_from_db(&conn).unwrap();
        assert!(runtime.get("good").is_some());
        assert!(runtime.get("bad").is_none());
    }

    fn valid_manifest_json(id: &str) -> String {
        serde_json::json!({
            "id": id,
            "name": "x",
            "version": "0.1.0",
            "entry": "index.js"
        })
        .to_string()
    }
}
