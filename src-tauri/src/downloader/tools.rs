//! バンドル / システムどちらの yt-dlp / ffmpeg を使うかの解決。
//!
//! Tauri の `bundle.externalBin` でアプリにバンドルされたバイナリを最優先。
//! 無ければシステム PATH にフォールバック。
//!
//! 解決順:
//! 1. `<resource_dir>/yt-dlp[.exe]` (Tauri bundle 配置先)
//! 2. `<exe_dir>/yt-dlp[.exe]`
//! 3. `<exe_dir>/binaries/yt-dlp[.exe]` (dev `cargo tauri dev` で隣に置かれる場合)
//! 4. `<src-tauri>/binaries/yt-dlp-<triple>[.exe]` (cargo run で workspace ルートから動かしたケース)
//! 5. PATH 検索 (`yt-dlp`)
//!
//! `Resolved::source` を使えばどこから取れたか UI に出せる。

use std::path::PathBuf;
use std::sync::OnceLock;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BinarySource {
    /// Tauri バンドルされたサイドカー
    Bundled,
    /// 実行ファイル隣 / dev 用 binaries フォルダ
    Sidecar,
    /// システム PATH
    SystemPath,
    /// 見つからなかった
    NotFound,
}

#[derive(Debug, Clone)]
pub struct Resolved {
    /// 実際に呼び出すコマンド (絶対パス or `"yt-dlp"` のような bare 名)
    pub command: String,
    pub source: BinarySource,
}

impl Resolved {
    pub fn not_found(name: &str) -> Self {
        Self {
            command: name.to_string(),
            source: BinarySource::NotFound,
        }
    }
}

/// `app` (Tauri AppHandle) を渡せばバンドル resource_dir も探す。
/// `None` なら exe_dir 周辺と PATH のみ。
pub fn resolve(app: Option<&tauri::AppHandle>, name: &str) -> Resolved {
    let exe_name = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    };

    // 1) resource_dir/yt-dlp(.exe)
    if let Some(app) = app {
        use tauri::Manager;
        if let Ok(resource_dir) = app.path().resource_dir() {
            let candidate = resource_dir.join(&exe_name);
            if candidate.is_file() {
                return Resolved {
                    command: candidate.to_string_lossy().into_owned(),
                    source: BinarySource::Bundled,
                };
            }
        }
    }

    // 2) <exe_dir>/yt-dlp
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let cand = dir.join(&exe_name);
            if cand.is_file() {
                return Resolved {
                    command: cand.to_string_lossy().into_owned(),
                    source: BinarySource::Sidecar,
                };
            }
            // 3) <exe_dir>/binaries/yt-dlp
            let cand2 = dir.join("binaries").join(&exe_name);
            if cand2.is_file() {
                return Resolved {
                    command: cand2.to_string_lossy().into_owned(),
                    source: BinarySource::Sidecar,
                };
            }
            // 4) workspace root の binaries/<name>-<triple> も探す (cargo run 用)
            //    target/<profile>/<exe> から 2 つ上が workspace ルート想定。
            if let Some(workspace) = dir.parent().and_then(|p| p.parent()) {
                if let Some(cand3) = find_with_triple_suffix(&workspace.join("binaries"), name) {
                    return Resolved {
                        command: cand3.to_string_lossy().into_owned(),
                        source: BinarySource::Sidecar,
                    };
                }
                // src-tauri/binaries/<name>-<triple> も
                if let Some(cand4) =
                    find_with_triple_suffix(&workspace.join("src-tauri").join("binaries"), name)
                {
                    return Resolved {
                        command: cand4.to_string_lossy().into_owned(),
                        source: BinarySource::Sidecar,
                    };
                }
            }
        }
    }

    // 5) PATH
    if which_in_path(name).is_some() {
        return Resolved {
            command: name.to_string(),
            source: BinarySource::SystemPath,
        };
    }

    Resolved::not_found(name)
}

fn find_with_triple_suffix(dir: &std::path::Path, name: &str) -> Option<PathBuf> {
    if !dir.is_dir() {
        return None;
    }
    let entries = std::fs::read_dir(dir).ok()?;
    let prefix = format!("{name}-");
    for e in entries.flatten() {
        let fname = e.file_name();
        let s = fname.to_string_lossy();
        if s.starts_with(&prefix) {
            let p = e.path();
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

fn which_in_path(name: &str) -> Option<PathBuf> {
    let exe_name = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    };
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let cand = dir.join(&exe_name);
        if cand.is_file() {
            return Some(cand);
        }
    }
    None
}

// ====== キャッシュ ======
// 起動中に何度も解決しないでよいように一度結果を取っておく。
static YTDLP_CACHE: OnceLock<Resolved> = OnceLock::new();
static FFMPEG_CACHE: OnceLock<Resolved> = OnceLock::new();

pub fn ytdlp(app: Option<&tauri::AppHandle>) -> Resolved {
    YTDLP_CACHE.get_or_init(|| resolve(app, "yt-dlp")).clone()
}

pub fn ffmpeg(app: Option<&tauri::AppHandle>) -> Resolved {
    FFMPEG_CACHE.get_or_init(|| resolve(app, "ffmpeg")).clone()
}

// ====== サブプロセス起動ヘルパ ======
// Windows では GUI アプリから素の `Command::new(...).output()` を呼ぶと
// 子プロセスごとにコンソールウィンドウが一瞬チラつく。設定画面の
// `get_app_info` は yt-dlp / ffmpeg の `--version` を毎回 2 本叩くため、
// 開くたびにターミナルが立ち上がりまくる挙動になっていた。
// `CREATE_NO_WINDOW` を付ければウィンドウを作らずに起動できる。Unix では no-op。

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// `tokio::process::Command::new` の代わりに使う。Windows ではコンソール
/// ウィンドウを抑制するフラグを立てる。
pub fn tokio_command<S: AsRef<std::ffi::OsStr>>(program: S) -> tokio::process::Command {
    #[cfg_attr(not(windows), allow(unused_mut))]
    let mut cmd = tokio::process::Command::new(program);
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd
}

/// `std::process::Command::new` の代わりに使う。Windows ではコンソール
/// ウィンドウを抑制するフラグを立てる。
pub fn std_command<S: AsRef<std::ffi::OsStr>>(program: S) -> std::process::Command {
    #[cfg_attr(not(windows), allow(unused_mut))]
    let mut cmd = std::process::Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}
