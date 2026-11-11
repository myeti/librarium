import { afterEach, describe, expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'

vi.mock('@tauri-apps/api/core', () => import('@/test/tauri'))
import TidyupReviewModal from './TidyupReviewModal.vue'
import { cleanup, keydown, mountUi } from '@/test/mountUi'
import type { Book, BookMetadata, TidyupSubmit } from '@/types'

const BOOK: Book = {
    id: 'b1', path: '/lib/Dune.epub', filename: 'Dune.epub', format: 'epub', sizeBytes: 1, dateAdded: '2026-01-01',
    metadata: { title: 'Dune', author: 'F. Herbert', categories: [] },
}

const FOUND: Partial<BookMetadata> = {
    title: 'Dune',
    author: 'Frank Herbert',
    isbn: '9780441013593',
    description: 'A desert planet.',
}

const top = (): HTMLDialogElement => {
    const all = document.querySelectorAll('dialog')
    return all[all.length - 1]
}
const applyButton = () => Array.from(top().querySelectorAll('button')).find(b => b.textContent?.includes('Apply'))!
const fieldRow = (label: string) => Array.from(top().querySelectorAll<HTMLElement>('.tidyup-field'))
    .find(r => r.querySelector('.field-label')?.textContent === label)!

function deferred() {
    let resolve!: () => void
    let reject!: (e: unknown) => void
    const promise = new Promise<void>((res, rej) => { resolve = res; reject = rej })
    return { promise, resolve, reject }
}

function open(props: Record<string, unknown> = {}) {
    return mountUi(TidyupReviewModal, {
        props: { book: BOOK, metadata: FOUND, source: 'openlibrary', ...props },
    })
}

describe('TidyupReviewModal', () => {
    afterEach(cleanup)

    it('is a labelled dialog that ticks what changed and offers the unchanged title unticked', () => {
        open()
        const title = document.getElementById(top().getAttribute('aria-labelledby')!)!
        expect(title.textContent).toContain('Metadata found online')
        expect(title.textContent).toContain('Open Library')
        expect(fieldRow('Author').querySelector('input')!.checked).toBe(true)
        expect(fieldRow('Author').textContent).toContain('was: F. Herbert')
        expect(fieldRow('Title').querySelector('input')!.checked).toBe(false)
        expect(fieldRow('Title').textContent).toContain('same as current')
        // Each checkbox is inside its label: clicking the row ticks it.
        expect(fieldRow('ISBN').querySelector('input')!.closest('label')).not.toBeNull()
        expect(applyButton().textContent).toContain('Apply 3 changes')
    })

    it('without `submit`, emits the ticked fields (the older contract)', async () => {
        const w = open()
        fieldRow('Description').querySelector('input')!.click()
        await flushPromises()
        expect(applyButton().textContent).toContain('Apply 2 changes')
        applyButton().click()
        await flushPromises()
        expect(w.emitted('apply')).toEqual([[{ author: 'Frank Herbert', isbn: '9780441013593' }, null]])
        expect(w.emitted('close')).toBeUndefined()
    })

    it('with `submit`: busy while it runs (Escape ignored), then closes', async () => {
        const saving = deferred()
        const submit = vi.fn(() => saving.promise)
        const w = open({ submit })
        applyButton().click()
        await flushPromises()
        expect(submit).toHaveBeenCalledWith({ author: 'Frank Herbert', isbn: '9780441013593', description: 'A desert planet.' }, null)
        expect(applyButton().getAttribute('aria-busy')).toBe('true')
        keydown(top(), 'Escape')
        expect(w.emitted('close')).toBeUndefined()
        saving.resolve()
        await flushPromises()
        expect(w.emitted('close')).toHaveLength(1)
        expect(w.emitted('apply')).toBeUndefined()
    })

    it('with `submit`: a failure keeps the review open with the reason', async () => {
        const w = open({ submit: () => Promise.reject(new Error('file is read-only')) })
        applyButton().click()
        await flushPromises()
        expect(w.emitted('close')).toBeUndefined()
        expect(top().querySelector('[role="alert"]')!.textContent).toContain('Could not save the metadata: file is read-only')
        // The choices are still there, to retry.
        expect(fieldRow('Author').querySelector('input')!.checked).toBe(true)
        expect(applyButton().hasAttribute('aria-busy')).toBe(false)
    })

    it('offers the proposed cover next to the current one, and never applies one that failed to load', async () => {
        const submit = vi.fn<TidyupSubmit>(() => Promise.resolve())
        open({ submit, coverUrl: 'https://covers.example/dune.jpg' })
        expect(applyButton().textContent).toContain('Apply 4 changes')
        top().querySelector<HTMLImageElement>('.cover-thumb.-new')!.dispatchEvent(new Event('error'))
        await flushPromises()
        expect(top().querySelector('.tidyup-cover-row')).toBeNull()
        expect(applyButton().textContent).toContain('Apply 3 changes')
        applyButton().click()
        await flushPromises()
        expect(submit.mock.calls[0][1]).toBeNull()
    })

    it('closes on Escape (nothing to lose)', () => {
        const w = open()
        keydown(top(), 'Escape')
        expect(w.emitted('close')).toHaveLength(1)
    })
})
