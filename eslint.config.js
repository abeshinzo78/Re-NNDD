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
    ignores: ['build/', '.svelte-kit/', 'dist/', 'src-tauri/target/', 'target/'],
  },
);
