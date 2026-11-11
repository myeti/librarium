/// <reference types="vite/client" />

declare const __BUILD_DATE__: string

/** Build-time settings (`.env`, see .env.example). All optional: a missing one disables its feature. */
interface ImportMetaEnv {
    /** The Librarium API (messages, error reports, feedback). */
    readonly VITE_API_URL?: string
    /** Sent to the Librarium API; compiled into the bundle, so not a secret. */
    readonly VITE_API_KEY?: string
    readonly VITE_UMAMI_URL?: string
    readonly VITE_UMAMI_WEBSITE_ID?: string
    /** "1" in the `mock` mode (`npm run vue:dev:mock`): use the fake backend. */
    readonly VITE_MOCK_BACKEND?: string
}

/**
 * `.vue` imports, for tools that don't understand single-file components.
 * vue-tsc resolves the real files first and type-checks them fully; this
 * fallback is what typescript-eslint sees (it type-checks each <script> on
 * its own), so imported components are a component type there instead of
 * an unresolved `any`. Named type exports therefore live in src/types, not
 * in .vue files.
 */
declare module '*.vue' {
    import type { DefineComponent } from 'vue'
    const component: DefineComponent
    export default component
}

declare module '*.svg?raw' {
    const content: string
    export default content
}

/** A Markdown file bundled as text (PRIVACY.md, shown by PrivacyPolicyDialog). */
declare module '*.md?raw' {
    const content: string
    export default content
}

declare module '*.svg?url' {
    const content: string
    export default content
}

declare module '*.svg' {
    const content: string
    export default content
}
