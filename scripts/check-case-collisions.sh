#!/usr/bin/env bash
# Case-insensitive filename collision detector for Svelte files.
#
# 背景:
#   Svelte 5 では runes を使うステートストアを `*.svelte.ts` ファイルに置く。
#   このファイルが同ディレクトリの `*.svelte` コンポーネントと case-insensitive
#   に衝突すると、Windows / macOS (NTFS / APFS デフォルト) では import 解決が
#   壊れる。例:
#
#     MiniPlayer.svelte      ← Svelte component
#     miniPlayer.svelte.ts   ← Svelte 5 store
#
#   Linux (case-sensitive) では `import './miniPlayer.svelte'` は
#   `miniPlayer.svelte.ts` に解決される (拡張子補完)。
#   Windows / macOS では `miniPlayer.svelte` が `MiniPlayer.svelte` に
#   case-insensitive に一致し、ストアではなくコンポーネントが選ばれて
#   `[MISSING_EXPORT]` で release build がコケる。
#
# 検出ルール:
#   git ls-files 上の `*.svelte` / `*.svelte.ts` / `*.svelte.js` を集めて
#   「lowercase 化 + 末尾の .ts/.js を剥がした key」が重複したらアウト。
#
# 失敗時は collision を列挙して exit 1。

set -euo pipefail

git ls-files | awk '
  /\.svelte(\.[jt]s)?$/ {
    key = tolower($0)
    sub(/\.[jt]s$/, "", key)
    if (key in seen && seen[key] != $0) {
      if (!err) {
        print "ERROR: case-insensitive filename collision between Svelte files."
        print "On case-insensitive filesystems (Windows / macOS), imports of the"
        print "shorter name will resolve to the wrong file:"
        print ""
      }
      print "  - " seen[key]
      print "    " $0
      print ""
      err = 1
    } else {
      seen[key] = $0
    }
  }
  END {
    if (err) {
      print "Rename one side so the basenames differ beyond case alone"
      print "(e.g. miniPlayer.svelte.ts -> miniPlayerStore.svelte.ts)."
      exit 1
    }
  }
'
