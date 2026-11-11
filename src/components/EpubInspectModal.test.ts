import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'

vi.mock('@tauri-apps/api/core', () => import('@/test/tauri'))
import { backendError, mockInvoke } from '@/test/tauri'
import EpubInspectModal from './EpubInspectModal.vue'
import { cleanup, keydown, mountUi } from '@/test/mountUi'
import type { EpubStructure } from '@/types'

const STRUCTURE: EpubStructure = {
    opfPath: 'OEBPS/content.opf',
    opfDir: 'OEBPS',
    coverDeclaration: { method: 'epub3', href: 'cover.jpg', mediaType: 'image/jpeg', itemId: 'cover' },
    manifest: [
        { id: 'cover', href: 'cover.jpg', mediaType: 'image/jpeg', properties: 'cover-image', zipPath: 'OEBPS/cover.jpg', exists: false },
        { id: 'ch1', href: 'ch1.xhtml', mediaType: 'application/xhtml+xml', properties: null, zipPath: 'OEBPS/ch1.xhtml', exists: true },
    ],
    spine: ['ch1'],
    zipEntries: [{ name: 'OEBPS/ch1.xhtml', size: 1200 }],
}

describe('EpubInspectModal', () => {
    beforeEach(() => { mockInvoke.reset() })
    afterEach(cleanup)

    it('shows the structure, with missing files said in words (not colour alone)', async () => {
        mockInvoke.on('inspect_epub', () => STRUCTURE)
        mountUi(EpubInspectModal, { props: { bookPath: '/lib/Dune.epub' } })
        const dialog = document.querySelector('dialog')!
        expect(document.getElementById(dialog.getAttribute('aria-labelledby')!)?.textContent).toContain('EPUB Inspector')
        await flushPromises()
        expect(dialog.textContent).toContain('OEBPS/content.opf')
        expect(Array.from(dialog.querySelectorAll('.zip-state')).map(s => s.textContent?.trim())).toEqual(['missing', 'yes'])
        expect(dialog.textContent).toContain('Missing')
    })

    it('says why it could not read the book, and closes on Escape', async () => {
        mockInvoke.on('inspect_epub', () => backendError({ code: 'INVALID_INPUT', message: 'not a zip archive' }))
        const w = mountUi(EpubInspectModal, { props: { bookPath: '/lib/Dune.epub' } })
        await flushPromises()
        const dialog = document.querySelector('dialog')!
        expect(dialog.querySelector('[role="alert"]')!.textContent).toContain('not a zip archive')
        keydown(dialog, 'Escape')
        expect(w.emitted('close')).toHaveLength(1)
    })
})
