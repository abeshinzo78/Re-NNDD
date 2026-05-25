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

use crate::plugins::manifest::{ManifestError, PluginManifest};

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
}

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
    let cursor = Cursor::new(zip_bytes);
    let mut archive = zip::ZipArchive::new(cursor)?;

    let manifest_text = read_manifest_text(&mut archive)?;
    let manifest = PluginManifest::parse_and_validate(&manifest_text, Some(app_version))?;

    // entry がアーカイブに存在することを確認
    let mut has_entry = false;
    for i in 0..archive.len() {
        let file = archive.by_index(i)?;
        let safe = safe_relative_path(&file)?;
        if safe.to_string_lossy() == manifest.entry {
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
    if target.exists() {
        if !replace {
            return Err(InstallError::AlreadyInstalled(manifest.id.clone()));
        }
        std::fs::remove_dir_all(&target)?;
    }

    let tmp = plugins_root.join(format!("{}.tmp-{}", manifest.id, std::process::id()));
    if tmp.exists() {
        std::fs::remove_dir_all(&tmp)?;
    }
    std::fs::create_dir_all(&tmp)?;

    // 全エントリを tmp に展開
    let extract_result: Result<(), InstallError> = (|| {
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
            std::io::copy(&mut file, &mut out)?;
        }
        Ok(())
    })();

    if let Err(e) = extract_result {
        let _ = std::fs::remove_dir_all(&tmp);
        return Err(e);
    }

    if let Err(e) = std::fs::rename(&tmp, &target) {
        let _ = std::fs::remove_dir_all(&tmp);
        return Err(InstallError::Io(e));
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
    if raw.contains("..")
        || raw.starts_with('/')
        || raw.starts_with('\\')
        || raw.contains(':')
    {
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
}
