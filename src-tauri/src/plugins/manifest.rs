//! プラグインの `manifest.json` 表現とバリデーション。
//!
//! ID/version/entry/permissions のいずれかが規約外の場合は
//! `InvalidManifest` を返す。受け入れたあとで Rust 側 dispatcher / フロント側
//! host がさらに踏み込んだ enforcement (permission チェック, key prefix 強制)
//! を行うが、ここではまず「壊れた JSON が DB に入らない」ことを保証する。

use std::collections::BTreeSet;

use semver::Version;
use serde::{Deserialize, Serialize};

/// プラグインに付与可能な権限の正式名一覧。
/// `permissions[]` にこれ以外の文字列があれば install 段階で拒否する。
pub const ALLOWED_PERMISSIONS: &[&str] = &[
    "net.fetch",
    "library.read",
    "settings.read",
    "settings.write",
    "notify",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginManifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub entry: String,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    #[serde(default, rename = "minAppVersion", skip_serializing_if = "Option::is_none")]
    pub min_app_version: Option<String>,
    #[serde(default)]
    pub permissions: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    #[error("manifest json parse failed: {0}")]
    Parse(String),
    #[error("invalid manifest: {0}")]
    Invalid(String),
    #[error("app version {app} does not satisfy plugin minAppVersion {required}")]
    AppTooOld { app: String, required: String },
}

impl PluginManifest {
    /// JSON 文字列からパース + バリデート。`app_version` は現在実行中のアプリ
    /// のバージョン (`env!("CARGO_PKG_VERSION")`)。`None` を渡すと minAppVersion
    /// チェックを skip する (テスト用)。
    pub fn parse_and_validate(
        json: &str,
        app_version: Option<&str>,
    ) -> Result<Self, ManifestError> {
        let m: PluginManifest = serde_json::from_str(json).map_err(|e| ManifestError::Parse(e.to_string()))?;
        m.validate(app_version)?;
        Ok(m)
    }

    pub fn validate(&self, app_version: Option<&str>) -> Result<(), ManifestError> {
        if !is_valid_plugin_id(&self.id) {
            return Err(ManifestError::Invalid(format!(
                "id {:?} does not match ^[a-z0-9][a-z0-9._-]{{2,63}}$",
                self.id
            )));
        }
        if self.name.is_empty() || self.name.chars().count() > 80 {
            return Err(ManifestError::Invalid(
                "name must be 1..=80 characters".into(),
            ));
        }
        Version::parse(&self.version)
            .map_err(|e| ManifestError::Invalid(format!("version not semver: {e}")))?;
        if let Some(min) = &self.min_app_version {
            let min_v = Version::parse(min)
                .map_err(|e| ManifestError::Invalid(format!("minAppVersion not semver: {e}")))?;
            if let Some(app) = app_version {
                let app_v = Version::parse(app).map_err(|e| {
                    ManifestError::Invalid(format!("app version not semver: {e}"))
                })?;
                if app_v < min_v {
                    return Err(ManifestError::AppTooOld {
                        app: app.to_string(),
                        required: min.clone(),
                    });
                }
            }
        }
        if !is_valid_entry(&self.entry) {
            return Err(ManifestError::Invalid(format!(
                "entry {:?} must be a relative .js/.mjs path with no '..' / '/' / '\\\\'",
                self.entry
            )));
        }
        if let Some(desc) = &self.description {
            if desc.chars().count() > 500 {
                return Err(ManifestError::Invalid("description > 500 chars".into()));
            }
        }
        if let Some(home) = &self.homepage {
            if !(home.starts_with("http://") || home.starts_with("https://")) {
                return Err(ManifestError::Invalid(
                    "homepage must be http(s) URL".into(),
                ));
            }
        }
        let allowed: BTreeSet<&'static str> = ALLOWED_PERMISSIONS.iter().copied().collect();
        for p in &self.permissions {
            if !allowed.contains(p.as_str()) {
                return Err(ManifestError::Invalid(format!(
                    "unknown permission {p:?}; allowed: {:?}",
                    ALLOWED_PERMISSIONS
                )));
            }
        }
        Ok(())
    }
}

/// id は ^[a-z0-9][a-z0-9._-]{2,63}$ 相当 (長さ 3..=64)。
/// regex をモジュール static に握らずに手書きでチェックすることで
/// clippy の OnceLock unwrap_used 警告を回避し、依存も減らす。
pub fn is_valid_plugin_id(s: &str) -> bool {
    let len = s.len();
    if !(3..=64).contains(&len) {
        return false;
    }
    let mut chars = s.chars();
    let first = match chars.next() {
        Some(c) => c,
        None => return false,
    };
    if !(first.is_ascii_lowercase() || first.is_ascii_digit()) {
        return false;
    }
    for c in chars {
        let ok = c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '_' || c == '-';
        if !ok {
            return false;
        }
    }
    true
}

/// entry ファイルパスとして安全か。
/// - 拡張子 .js / .mjs
/// - `..` / `/` / `\` / `:` (Windows drive) を含まない
/// - 空でない
fn is_valid_entry(s: &str) -> bool {
    if s.is_empty() || s.len() > 200 {
        return false;
    }
    if s.contains("..") || s.contains('/') || s.contains('\\') || s.contains(':') {
        return false;
    }
    s.ends_with(".js") || s.ends_with(".mjs")
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn good_manifest_json() -> String {
        serde_json::json!({
            "id": "com.example.demo",
            "name": "Demo",
            "version": "0.1.0",
            "entry": "index.js"
        })
        .to_string()
    }

    #[test]
    fn happy_path_parses() {
        let m = PluginManifest::parse_and_validate(&good_manifest_json(), None).unwrap();
        assert_eq!(m.id, "com.example.demo");
    }

    #[test]
    fn rejects_bad_id() {
        let j = serde_json::json!({
            "id": "BAD ID",
            "name": "x",
            "version": "0.1.0",
            "entry": "index.js"
        })
        .to_string();
        assert!(PluginManifest::parse_and_validate(&j, None).is_err());
    }

    #[test]
    fn rejects_non_semver_version() {
        let j = serde_json::json!({
            "id": "good.id",
            "name": "x",
            "version": "not-semver",
            "entry": "index.js"
        })
        .to_string();
        assert!(PluginManifest::parse_and_validate(&j, None).is_err());
    }

    #[test]
    fn rejects_path_traversal_entry() {
        let j = serde_json::json!({
            "id": "good.id",
            "name": "x",
            "version": "0.1.0",
            "entry": "../etc/passwd"
        })
        .to_string();
        assert!(PluginManifest::parse_and_validate(&j, None).is_err());
    }

    #[test]
    fn rejects_unknown_permission() {
        let j = serde_json::json!({
            "id": "good.id",
            "name": "x",
            "version": "0.1.0",
            "entry": "index.js",
            "permissions": ["evil"]
        })
        .to_string();
        assert!(PluginManifest::parse_and_validate(&j, None).is_err());
    }

    #[test]
    fn rejects_when_app_too_old() {
        let j = serde_json::json!({
            "id": "good.id",
            "name": "x",
            "version": "0.1.0",
            "entry": "index.js",
            "minAppVersion": "999.0.0"
        })
        .to_string();
        let err = PluginManifest::parse_and_validate(&j, Some("0.1.0")).unwrap_err();
        matches!(err, ManifestError::AppTooOld { .. });
    }

    #[test]
    fn accepts_only_allowed_permissions() {
        let j = serde_json::json!({
            "id": "good.id",
            "name": "x",
            "version": "0.1.0",
            "entry": "index.js",
            "permissions": ["net.fetch", "library.read", "settings.read", "settings.write", "notify"]
        })
        .to_string();
        assert!(PluginManifest::parse_and_validate(&j, None).is_ok());
    }

    #[test]
    fn rejects_entry_without_js_extension() {
        let j = serde_json::json!({
            "id": "good.id",
            "name": "x",
            "version": "0.1.0",
            "entry": "index.html"
        })
        .to_string();
        assert!(PluginManifest::parse_and_validate(&j, None).is_err());
    }
}
