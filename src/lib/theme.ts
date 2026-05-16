// アプリのテーマ定義。
//
// 各テーマは `<html data-theme="...">` 上に展開する CSS 変数の集合。
// `applyTheme(id)` でグローバルに適用する。設定 `appearance.theme` と連動して
// `+layout.svelte` の onMount/$effect で呼ばれる。
//
// 変数は `:global([data-theme="..."]) { ... }` で +layout.svelte 側に静的にも
// 書いてあるが、SSR / 初期描画前にも反映できるよう JS でも投入する。
//
// テーマを増やす場合: 下の THEMES に追加 → 自動で設定画面の選択肢に出る。

export type ThemeId = 'niconico' | 'dark' | 'light';

export type ThemeVars = Record<string, string>;

/**
 * 「ニコニコ風」テーマ。スクリーンショットを正確に再現した既定値。
 * 純黒背景 + 濃いグレーサーフェス + ニコニコ系の青アクセント。
 */
const NICONICO_VARS: ThemeVars = {
  '--color-scheme': 'dark',
  // 背景レイヤー
  '--bg': '#000000',
  '--surface': '#121212',
  '--surface-2': '#161616',
  '--surface-3': '#1a1a1a',
  '--surface-hover': '#1f1f1f',
  '--surface-active': '#2a2a2a',
  '--input-bg': '#0f0f0f',
  '--code-bg': '#0a0a0a',
  // 境界線
  '--border': '#1f1f1f',
  '--border-2': '#2a2a2a',
  '--border-3': '#2f2f2f',
  '--border-strong': '#3a3a3a',
  // テキスト
  '--text': '#eaeaea',
  '--text-2': '#cfcfcf',
  '--text-3': '#b0b0b0',
  '--text-muted': '#9a9a9a',
  '--text-dim': '#6a6a6a',
  '--text-faint': '#555555',
  '--text-heading': '#f5f5f5',
  // アクセント (ニコニコ青)
  '--accent': '#2563eb',
  '--accent-hover': '#3b78f0',
  '--accent-text': '#ffffff',
  '--accent-soft-bg': 'rgba(37, 99, 235, 0.15)',
  '--accent-soft-border': '#2a3f5a',
  '--accent-soft-text': '#c5d8f5',
  // リンク
  '--link': '#6ea8fe',
  '--link-strong': '#93c5fd',
  // バッジ (青系)
  '--badge-blue-bg': '#1f2a44',
  '--badge-blue-text': '#93c5fd',
  '--badge-blue-bg-soft': '#1a2a44',
  '--badge-blue-border': '#2a3f5a',
  // 成功 (緑)
  '--success-bg': '#102d20',
  '--success-text': '#bbf7d0',
  '--success-border': '#1e6b48',
  '--success-strong': '#4ade80',
  '--success-icon-bg': '#1a3a26',
  '--success-icon-text': '#b3f5b3',
  '--success-icon-border': '#2a5a3a',
  // 警告 (黄)
  '--warn-bg': '#2a2410',
  '--warn-text': '#fde68a',
  '--warn-border': '#5a4a1a',
  // エラー (赤)
  '--error-bg': '#2a1212',
  '--error-text': '#f5b3b3',
  '--error-border': '#5a2222',
  '--error-strong': '#f87171',
  '--error-hover-bg': '#3a1a1a',
  // タグ
  '--tag-bg': '#1f1f1f',
  '--tag-text': '#c0c0c0',
  '--tag-hover-bg': '#2a2a2a',
  '--tag-hover-text': '#ffffff',
  '--tag-hover-border': '#3a3a3a',
  '--tag-locked-bg': '#1e2a3a',
  '--tag-locked-text': '#b3c5ff',
  '--tag-locked-hover-bg': '#243246',
  // メニュー (ドロップダウン)
  '--menu-bg': '#181818',
  '--menu-border': '#333333',
  '--menu-shadow': '0 8px 24px rgba(0, 0, 0, 0.6)',
  // スクロールバー / オーナー
  '--owner-card-bg': '#161616',
  // 連続再生インジケーター
  '--resume-bg-overlay': 'rgba(0, 0, 0, 0.7)',
  '--resume-bar': '#333333',
  '--resume-bar-fill': '#93c5fd',
  '--resume-text': '#c5d8f5',
  // セレクトオプション (popup) - light で常時白背景
  '--select-option-bg': '#ffffff',
  '--select-option-text': '#000000',
};

/**
 * 「ダーク」テーマ。`niconico` と同色 (既定値が `dark` だったときの
 * 後方互換用 + 「シンプルなダーク」として残す)。
 */
const DARK_VARS: ThemeVars = { ...NICONICO_VARS };

/**
 * 「ライト」テーマ。背景白 + 濃い文字 + ニコニコ青アクセント。
 * ダークと切替えると視覚的に明確に変化する。
 */
const LIGHT_VARS: ThemeVars = {
  '--color-scheme': 'light',
  // 背景レイヤー
  '--bg': '#f5f7fa',
  '--surface': '#ffffff',
  '--surface-2': '#ffffff',
  '--surface-3': '#f3f5f8',
  '--surface-hover': '#e9edf2',
  '--surface-active': '#dde3eb',
  '--input-bg': '#ffffff',
  '--code-bg': '#f1f3f7',
  // 境界線
  '--border': '#e2e6ec',
  '--border-2': '#d3d8e0',
  '--border-3': '#c8cdd5',
  '--border-strong': '#b3b9c2',
  // テキスト
  '--text': '#1f2937',
  '--text-2': '#374151',
  '--text-3': '#4b5563',
  '--text-muted': '#6b7280',
  '--text-dim': '#9ca3af',
  '--text-faint': '#b9bfc8',
  '--text-heading': '#111827',
  // アクセント
  '--accent': '#2563eb',
  '--accent-hover': '#1d4ed8',
  '--accent-text': '#ffffff',
  '--accent-soft-bg': 'rgba(37, 99, 235, 0.12)',
  '--accent-soft-border': '#bfd2f5',
  '--accent-soft-text': '#1e40af',
  // リンク
  '--link': '#1d4ed8',
  '--link-strong': '#1e40af',
  // バッジ
  '--badge-blue-bg': '#dbeafe',
  '--badge-blue-text': '#1d4ed8',
  '--badge-blue-bg-soft': '#e0eaff',
  '--badge-blue-border': '#bfd2f5',
  // 成功
  '--success-bg': '#dcfce7',
  '--success-text': '#065f46',
  '--success-border': '#86efac',
  '--success-strong': '#16a34a',
  '--success-icon-bg': '#d1fae5',
  '--success-icon-text': '#065f46',
  '--success-icon-border': '#6ee7b7',
  // 警告
  '--warn-bg': '#fef3c7',
  '--warn-text': '#854d0e',
  '--warn-border': '#fcd34d',
  // エラー
  '--error-bg': '#fee2e2',
  '--error-text': '#991b1b',
  '--error-border': '#fca5a5',
  '--error-strong': '#dc2626',
  '--error-hover-bg': '#fecaca',
  // タグ
  '--tag-bg': '#eef0f4',
  '--tag-text': '#374151',
  '--tag-hover-bg': '#dde2e8',
  '--tag-hover-text': '#111827',
  '--tag-hover-border': '#c8cdd5',
  '--tag-locked-bg': '#e0eaff',
  '--tag-locked-text': '#1e40af',
  '--tag-locked-hover-bg': '#cad9fa',
  // メニュー
  '--menu-bg': '#ffffff',
  '--menu-border': '#d3d8e0',
  '--menu-shadow': '0 8px 24px rgba(15, 23, 42, 0.15)',
  // オーナー
  '--owner-card-bg': '#ffffff',
  // 連続再生
  '--resume-bg-overlay': 'rgba(15, 23, 42, 0.55)',
  '--resume-bar': '#cbd5e1',
  '--resume-bar-fill': '#1d4ed8',
  '--resume-text': '#e0eaff',
  // セレクトオプション
  '--select-option-bg': '#ffffff',
  '--select-option-text': '#000000',
};

export const THEMES: Record<ThemeId, { label: string; description?: string; vars: ThemeVars }> = {
  niconico: {
    label: 'ニコニコ風',
    description: '純黒背景 + ニコニコ青アクセント (スクリーンショット再現)',
    vars: NICONICO_VARS,
  },
  dark: {
    label: 'ダーク',
    description: 'ニコニコ風と同系統のシンプルなダーク',
    vars: DARK_VARS,
  },
  light: {
    label: 'ライト',
    description: '白背景 + 濃い文字 + 青アクセント',
    vars: LIGHT_VARS,
  },
};

export const DEFAULT_THEME: ThemeId = 'niconico';

/** 指定 ID のテーマを <html> に適用。未知 ID は既定にフォールバック。 */
export function applyTheme(id: string): void {
  if (typeof document === 'undefined') return;
  const theme = THEMES[id as ThemeId] ?? THEMES[DEFAULT_THEME];
  const resolvedId = (id as ThemeId) in THEMES ? id : DEFAULT_THEME;
  const root = document.documentElement;
  root.setAttribute('data-theme', resolvedId);
  for (const [k, v] of Object.entries(theme.vars)) {
    root.style.setProperty(k, v);
  }
}
