import js from '@eslint/js';
import ts from 'typescript-eslint';
import svelte from 'eslint-plugin-svelte';
import prettier from 'eslint-config-prettier';
import globals from 'globals';

export default ts.config(
  js.configs.recommended,
  ...ts.configs.recommended,
  ...svelte.configs['flat/recommended'],
  prettier,
  ...svelte.configs['flat/prettier'],
  {
    languageOptions: {
      globals: { ...globals.browser, ...globals.node },
    },
    rules: {
      // `_` 始まりの引数/変数は「意図的に未使用」とみなす慣用に合わせる。
      '@typescript-eslint/no-unused-vars': [
        'error',
        { argsIgnorePattern: '^_', varsIgnorePattern: '^_', caughtErrorsIgnorePattern: '^_' },
      ],
    },
  },
  {
    files: ['**/*.svelte'],
    languageOptions: {
      parserOptions: { parser: ts.parser },
    },
    rules: {
      'svelte/no-navigation-without-resolve': 'off',
    },
  },
  {
    files: ['**/*.svelte.ts'],
    languageOptions: {
      parserOptions: { parser: ts.parser },
    },
    rules: {
      // `.svelte.ts` モジュールから `goto()` を呼ぶケース (コマンドパレット等) は
      // 通常の string href で十分。`.svelte` 本体と同じく off にする。
      'svelte/no-navigation-without-resolve': 'off',
    },
  },
  {
    // `scripts/burnin-verify/` は焼き込みの実機検証ハーネス (Node + @napi-rs/canvas)。
    // アプリ本体ではなく検証専用ツールなので、アプリ用 lint ルールの対象外にする。
    ignores: [
      'build/',
      '.svelte-kit/',
      'dist/',
      'src-tauri/target/',
      'target/',
      'scripts/burnin-verify/',
    ],
  },
);
