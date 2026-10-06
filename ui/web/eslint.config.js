import js from '@eslint/js'
import globals from 'globals'
import reactHooks from 'eslint-plugin-react-hooks'
import reactRefresh from 'eslint-plugin-react-refresh'
import tseslint from 'typescript-eslint'
import { defineConfig, globalIgnores } from 'eslint/config'

// 这份配置原先根本不存在：依赖全在 devDependencies 里躺着，`npm run lint`
// 却因为找不到配置而直接退 2。补上它不需要新增任何依赖。
//
// 规则集不自己挑，用社区已经调好的：`js.configs.recommended` +
// `tseslint.configs.recommended` + react-hooks + react-refresh。
// react-hooks / react-refresh 的 `configs.recommended` 是旧式 eslintrc 形状
// （`plugins` 是字符串数组），flat config 不收，所以直接挂插件对象再展开它的
// rules —— 这也是 ESLint 10 迁移指南给的写法。
export default defineConfig([
  globalIgnores(['dist']),
  {
    files: ['**/*.{ts,tsx}'],
    extends: [js.configs.recommended, tseslint.configs.recommended],
    plugins: {
      'react-hooks': reactHooks,
      'react-refresh': reactRefresh,
    },
    rules: {
      ...reactHooks.configs.flat.recommended.rules,
      'react-refresh/only-export-components': ['warn', { allowConstantExport: true }],
    },
    languageOptions: {
      ecmaVersion: 2020,
      globals: globals.browser,
    },
  },
])
