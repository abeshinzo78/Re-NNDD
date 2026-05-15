#!/usr/bin/env bash
# yt-dlp と ffmpeg のスタンドアロンバイナリを取得して
# src-tauri/binaries/<name>-<triple> に配置する。
#
# Tauri の `bundle.externalBin` 機構が triple サフィックス前提なので、
# このスクリプトもそれに合わせる。
#
# 使い方:
#   bash scripts/fetch-binaries.sh           # 公式 single-file をダウンロード
#   bash scripts/fetch-binaries.sh --system  # システム PATH の物を symlink (dev 用)
#
# 配布バンドル (`tauri build`) を作るときは引数なしの DL モードを推奨。

set -euo pipefail

MODE="download"
if [[ "${1:-}" == "--system" ]]; then
  MODE="symlink"
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
BIN_DIR="$ROOT/src-tauri/binaries"
mkdir -p "$BIN_DIR"

# target triple を決定する。
# 1) 環境変数 TARGET_TRIPLE が指定されていればそれを使う (CI matrix からの注入用)。
# 2) なければ `rustc -vV` の host から拾う (ローカル開発用)。
#
# CI で rustup の PATH 反映タイミングに依存させないため、matrix 値を優先する
# ロジックにしておく (macos-14 / aarch64 runner で rustc を spawn できず
# build がコケた実績がある)。
TRIPLE="${TARGET_TRIPLE:-}"
if [[ -z "$TRIPLE" ]]; then
  if command -v rustc >/dev/null 2>&1; then
    TRIPLE="$(rustc -vV 2>/dev/null | awk '/^host:/ {print $2}')"
  fi
fi
if [[ -z "$TRIPLE" ]]; then
  echo "error: target triple を解決できません" >&2
  echo "       環境変数 TARGET_TRIPLE を指定するか、rustc を PATH に通してください" >&2
  exit 1
fi
echo "==> target triple: $TRIPLE"

# プラットフォーム判定
case "$TRIPLE" in
  x86_64-unknown-linux-gnu)   YTDLP_URL="https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp_linux";       FFMPEG_FAMILY="linux64" ;;
  x86_64-unknown-linux-musl)  YTDLP_URL="https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp_linux";       FFMPEG_FAMILY="linux64" ;;
  aarch64-unknown-linux-gnu)  YTDLP_URL="https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp_linux_aarch64"; FFMPEG_FAMILY="linuxarm64" ;;
  x86_64-apple-darwin)        YTDLP_URL="https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp_macos";       FFMPEG_FAMILY="osx64" ;;
  aarch64-apple-darwin)       YTDLP_URL="https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp_macos";       FFMPEG_FAMILY="osxarm64" ;;
  x86_64-pc-windows-msvc)     YTDLP_URL="https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp.exe";         FFMPEG_FAMILY="win64" ;;
  *)
    echo "error: 未対応の triple: $TRIPLE" >&2
    exit 1
    ;;
esac

# Windows なら .exe サフィックス
EXT=""
[[ "$TRIPLE" == *"windows"* ]] && EXT=".exe"

YTDLP_DST="$BIN_DIR/yt-dlp-$TRIPLE$EXT"
FFMPEG_DST="$BIN_DIR/ffmpeg-$TRIPLE$EXT"

fetch() {
  local url="$1" out="$2"
  if command -v curl >/dev/null 2>&1; then
    curl -fL --progress-bar -o "$out" "$url"
  elif command -v wget >/dev/null 2>&1; then
    wget -qO "$out" "$url"
  else
    echo "error: curl も wget も無い" >&2
    exit 1
  fi
}

link_system() {
  local name="$1" dst="$2"
  local src
  src="$(command -v "$name" || true)"
  if [[ -z "$src" ]]; then
    echo "error: system PATH に $name が見つからない (apt install $name など)" >&2
    return 1
  fi
  # 実体を resolve (シンボリックリンク追跡)
  src="$(readlink -f "$src" 2>/dev/null || echo "$src")"
  ln -sf "$src" "$dst"
  echo "    symlinked $name: $src -> $dst"
}

# === yt-dlp ===
if [[ -x "$YTDLP_DST" ]] && [[ ! -L "$YTDLP_DST" ]]; then
  echo "==> yt-dlp: 既に存在 ($YTDLP_DST) — skip"
elif [[ "$MODE" == "symlink" ]]; then
  echo "==> yt-dlp: PATH から symlink"
  link_system "yt-dlp" "$YTDLP_DST"
else
  echo "==> yt-dlp: 取得中..."
  fetch "$YTDLP_URL" "$YTDLP_DST"
  chmod +x "$YTDLP_DST"
  echo "    -> $YTDLP_DST"
fi

# === ffmpeg ===
# 静的ビルドの取り出し場所がプラットフォーム毎に違う。
# Linux  : https://johnvansickle.com/ffmpeg/  (.tar.xz, ffmpeg/ 内)
# macOS  : https://evermeet.cx/ffmpeg/        (.zip, ffmpeg を直含)
# Windows: https://www.gyan.dev/ffmpeg/builds/ (.7z 多いが zip もあり)
if [[ -x "$FFMPEG_DST" ]] && [[ ! -L "$FFMPEG_DST" ]]; then
  echo "==> ffmpeg: 既に存在 ($FFMPEG_DST) — skip"
elif [[ "$MODE" == "symlink" ]]; then
  echo "==> ffmpeg: PATH から symlink"
  link_system "ffmpeg" "$FFMPEG_DST"
else
  TMP="$(mktemp -d)"
  # SC2064: $TMP を登録時点で固定展開させる意図 (子プロセスや変数再代入の影響を避ける)
  # shellcheck disable=SC2064
  trap "rm -rf '$TMP'" EXIT

  case "$FFMPEG_FAMILY" in
    linux64)
      # BtbN の GitHub release は安定して取れる。johnvansickle は TLS が
      # たまにこける。
      URL="https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-linux64-gpl.tar.xz"
      echo "==> ffmpeg(linux64): 取得中 (BtbN)..."
      fetch "$URL" "$TMP/ffmpeg.tar.xz"
      tar -xJf "$TMP/ffmpeg.tar.xz" -C "$TMP"
      cp "$(find "$TMP" -type f -name ffmpeg -executable | head -n 1)" "$FFMPEG_DST"
      chmod +x "$FFMPEG_DST"
      ;;
    linuxarm64)
      URL="https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-linuxarm64-gpl.tar.xz"
      echo "==> ffmpeg(linuxarm64): 取得中 (BtbN)..."
      fetch "$URL" "$TMP/ffmpeg.tar.xz"
      tar -xJf "$TMP/ffmpeg.tar.xz" -C "$TMP"
      cp "$(find "$TMP" -type f -name ffmpeg -executable | head -n 1)" "$FFMPEG_DST"
      chmod +x "$FFMPEG_DST"
      ;;
    osx64|osxarm64)
      URL="https://evermeet.cx/ffmpeg/getrelease/zip"
      echo "==> ffmpeg(macOS): 取得中..."
      fetch "$URL" "$TMP/ffmpeg.zip"
      unzip -q "$TMP/ffmpeg.zip" -d "$TMP"
      cp "$TMP/ffmpeg" "$FFMPEG_DST"
      chmod +x "$FFMPEG_DST"
      ;;
    win64)
      URL="https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-essentials.zip"
      echo "==> ffmpeg(win64): 取得中..."
      fetch "$URL" "$TMP/ffmpeg.zip"
      unzip -q "$TMP/ffmpeg.zip" -d "$TMP"
      cp "$(find "$TMP" -type f -name ffmpeg.exe | head -n 1)" "$FFMPEG_DST"
      ;;
  esac
  echo "    -> $FFMPEG_DST"
fi

# サイズ表示
echo
echo "==> 完了"
ls -lh "$BIN_DIR" | awk '{print "    "$0}'
echo
echo "次回からアプリ起動時にバンドルされたバイナリが優先で使われます。"
echo "ビルドする場合は \`npm run tauri build\` でこのバイナリも .deb / .app /"
echo ".msi に取り込まれます。"
