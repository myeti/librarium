import { convertFileSrc } from '@tauri-apps/api/core'
import type { CoverInfo } from '@/types'

/**
 * Covers are files in `<library>/.covers/` (see docs/IPC.md, `ensure_cover`),
 * shown through Tauri's asset protocol: never data URLs in reactive state.
 * The library store keeps them by book id (`libraryStore.coverOf(id)`).
 */

/**
 * The URL to show a cover, or `null` when there is none (yet).
 * `coverVersion` (the file's modification time) is appended so a replaced
 * cover, which keeps the same path, isn't served from the webview's cache.
 */
export function coverUrl(cover: Partial<CoverInfo> | null | undefined): string | null {
    if (!cover?.coverPath) { return null }
    return `${convertFileSrc(cover.coverPath)}?v=${cover.coverVersion ?? 0}`
}
