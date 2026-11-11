import { defineConfig, mergeConfig } from 'vitest/config'
import viteConfig from './vite.config.ts'

/**
 * Unit tests run in happy-dom so stores, services and components can use DOM
 * APIs. Tauri is not available in tests: mock `@tauri-apps/api/core`'s
 * `invoke` with `vi.mock` where a test needs it (see src/test/tauri.ts).
 */
export default mergeConfig(viteConfig, defineConfig({
    test: {
        environment: 'happy-dom',
        include: ['src/**/*.test.ts', 'scripts/**/*.test.mjs'],
        restoreMocks: true,
    },
}))
