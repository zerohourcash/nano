import js from '@eslint/js'
import globals from 'globals'
import reactHooks from 'eslint-plugin-react-hooks'
import reactRefresh from 'eslint-plugin-react-refresh'
import tseslint from 'typescript-eslint'
import { defineConfig, globalIgnores } from 'eslint/config'

export default defineConfig([
  globalIgnores(['dist']),
  {
    files: ['**/*.{ts,tsx}'],
    extends: [
      js.configs.recommended,
      tseslint.configs.recommended,
      reactHooks.configs.flat.recommended,
      reactRefresh.configs.vite,
    ],
    languageOptions: {
      ecmaVersion: 2020,
      globals: globals.browser,
    },
  },
  {
    files: ['src/pages/CreateTool.tsx'],
    rules: {
      // React Hook Form's watch API is intentionally external to React Compiler.
      'react-hooks/incompatible-library': 'off',
    },
  },
  {
    files: [
      'src/components/ui/**/*.{ts,tsx}',
      'src/pages/admin/ui.tsx',
      'src/providers/trpc.tsx',
      'src/lib/store.tsx',
    ],
    rules: {
      // These modules intentionally colocate components with reusable helpers.
      'react-refresh/only-export-components': 'off',
    },
  },
])
