// ESLint for the Vue frontend (`npx eslint src`).
//
// - eslint-plugin-vue `flat/recommended`: the full Vue 3 rule set (the old
//   config spread the legacy `recommended` object, which only enabled ~8 rules).
// - typescript-eslint `recommendedTypeChecked`, with the project service so
//   rules can use type information in both `.ts` files and `<script lang="ts">`
//   blocks. Promise misuse is an error: a forgotten `await` silently drops a
//   backend failure, which is exactly the kind of bug this app can't afford.
// - Conventions (SPECS.md, "Conventions"): Options API components only
//   (decision D5), 4-space indent, no semicolons, braces on every block.
import { defineConfig } from 'eslint/config'
import pluginVue from 'eslint-plugin-vue'
import tseslint from 'typescript-eslint'
import vueParser from 'vue-eslint-parser'

export default defineConfig(
    { ignores: ['dist/**', 'src-tauri/**', 'node_modules/**'] },

    ...tseslint.configs.recommendedTypeChecked,
    ...pluginVue.configs['flat/recommended'],

    // Type information for every linted file, .vue included.
    {
        files: ['**/*.ts', '**/*.vue'],
        languageOptions: {
            parserOptions: {
                projectService: true,
                tsconfigRootDir: import.meta.dirname,
                extraFileExtensions: ['.vue'],
            },
        },
        rules: {
            '@typescript-eslint/no-floating-promises': 'error',
            '@typescript-eslint/no-misused-promises': 'error',
            '@typescript-eslint/no-explicit-any': 'error',
            // `_unused` parameters are a deliberate way to document a signature.
            '@typescript-eslint/no-unused-vars': ['error', { argsIgnorePattern: '^_', varsIgnorePattern: '^_' }],
            'semi': ['error', 'never'],
            'curly': ['error', 'all'],
        },
    },

    // .vue files: vue-eslint-parser for the template, typescript-eslint for <script lang="ts">.
    {
        files: ['**/*.vue'],
        languageOptions: {
            parser: vueParser,
            parserOptions: {
                parser: tseslint.parser,
                sourceType: 'module',
            },
        },
        rules: {
            // Decision D5: Options API everywhere (no setup(), no <script setup>).
            'vue/component-api-style': ['error', ['options']],
            'vue/html-indent': ['error', 4],
            'indent': 'off', // vue/script-indent handles indentation in .vue files
            'vue/script-indent': ['error', 4, { baseIndent: 1, switchCase: 1 }],
            'vue/html-self-closing': ['error', { html: { void: 'always', normal: 'never', component: 'always' } }],
            'vue/multi-word-component-names': 'off',
            // v-html is only allowed for bundled SVG assets, each with a targeted
            // disable comment saying so: server or book text must never reach it.
            'vue/no-v-html': 'error',
            // The house style keeps short elements and their attributes on one
            // line; these two layout rules of `flat/recommended` would rewrite
            // nearly every template for no gain in correctness.
            'vue/max-attributes-per-line': 'off',
            'vue/singleline-html-element-content-newline': 'off',
            // Vue binds every function in `methods` to the component instance,
            // so passing `this.onKeyDown` to addEventListener is safe — the
            // situation this rule guards against can't happen in Options API
            // components.
            '@typescript-eslint/unbound-method': 'off',
        },
    },

    {
        files: ['**/*.ts'],
        rules: {
            'indent': ['error', 4],
        },
    },
)
