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
      globals: globals.browser,
    },
    rules: {
      // A leading underscore is this codebase's existing way of saying "this
      // binding is deliberately unused" — a mock whose typed signature documents
      // the shape it stands in for, a destructured field skipped to reach a later
      // one, a caught error nobody inspects. The rule's defaults flag those
      // anyway, so every such site had to either be rewritten or carry an
      // inline disable.
      //
      // This declares the convention rather than relaxing the rule: a name with
      // no underscore that goes unused is still an error, which is the case that
      // actually catches dead code.
      '@typescript-eslint/no-unused-vars': [
        'error',
        {
          argsIgnorePattern: '^_',
          varsIgnorePattern: '^_',
          caughtErrorsIgnorePattern: '^_',
        },
      ],
    },
  },
])
