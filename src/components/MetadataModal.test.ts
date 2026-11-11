import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'

vi.mock('@tauri-apps/api/core', () => import('@/test/tauri'))
import MetadataModal from './MetadataModal.vue'
import { useLibraryStore } from '@/stores/library'
import { cleanup, keydown, mountUi, testPinia } from '@/test/mountUi'
import type { Book, BookMetadata, CoverChange } from '@/types'

/** MetadataModal's `submit` prop (MetadataSubmit; types exported from .vue files aren't visible to lint). */
type Submit = (bookIds: string[], metadata: Partial<BookMetadata>, cover: CoverChange, seriesIndexes?: Record<string, number>) => Promise<void>
const resolvingSubmit = () => vi.fn<Submit>(() => Promise.resolve())

const books: Book[] = ['a', 'b'].map(id => ({
    id, path: `/lib/${id}.epub`, filename: `${id}.epub`, format: 'epub', sizeBytes: 1, dateAdded: '2026-01-01',
    metadata: { title: `Title ${id}`, author: `Author ${id}`, series: 'Saga', seriesIndex: 2, categories: [] },
}))

const dialogs = () => Array.from(document.querySelectorAll('dialog'))
const top = () => dialogs()[dialogs().length - 1]
const form = () => document.querySelector<HTMLFormElement>('#metadata-form')!
const fieldLabelled = (label: string) => {
    const l = Array.from(document.querySelectorAll('label')).find(el => el.textContent?.trim() === label)!
    return document.getElementById(l.htmlFor) as HTMLInputElement
}

function type(input: HTMLInputElement | HTMLTextAreaElement, value: string) {
    input.value = value
    input.dispatchEvent(new Event('input'))
}

function deferred() {
    let resolve!: () => void
    let reject!: (e: unknown) => void
    const promise = new Promise<void>((res, rej) => { resolve = res; reject = rej })
    return { promise, resolve, reject }
}

describe('MetadataModal', () => {
    let opener: HTMLButtonElement

    beforeEach(() => {
        opener = document.createElement('button')
        document.body.appendChild(opener)
        opener.focus()
    })

    afterEach(cleanup)

    function open(submit: Submit, list = books) {
        const wrapper = mountUi(MetadataModal, { props: { books: list, submit } })
        return wrapper
    }

    it('sends only the filled-in fields of a batch edit, and closes once saved', async () => {
        const submit = resolvingSubmit()
        const w = open(submit)
        type(form().querySelector('input')!, 'Same Author')   // first field in batch mode
        form().dispatchEvent(new Event('submit'))
        await flushPromises()

        expect(submit).toHaveBeenCalledWith(['a', 'b'], { author: 'Same Author' }, { action: 'none', base64: null }, undefined)
        expect(w.emitted('close')).toHaveLength(1)
    })

    describe('a batch: the books in an order, and their volumes', () => {
        const book = (id: string, title: string, seriesIndex?: number): Book => ({
            id, path: `/lib/${id}.epub`, filename: `${id}.epub`, format: 'epub', sizeBytes: 1, dateAdded: '2026-01-01',
            metadata: { title, author: 'Frank Herbert', series: seriesIndex ? 'Dune' : undefined, seriesIndex, categories: [] },
        })
        const trilogy = [book('c', 'Children of Dune', 3), book('x', 'An Extra'), book('a', 'Dune', 1), book('b', 'Dune Messiah', 2)]
        const listed = () => Array.from(document.querySelectorAll('.batch-order__title')).map(el => el.textContent?.trim())
        const numbers = () => Array.from(document.querySelectorAll('.batch-order__number')).map(el => el.textContent?.trim())
        const numbering = () => Array.from(document.querySelectorAll<HTMLInputElement>('input[type="checkbox"]'))
            .find(i => i.closest('.numbering'))!

        it('opens with each series by volume, and the books outside one last', () => {
            const mixed = [
                book('f2', 'Foundation and Empire', 2), book('x', 'An Extra'), book('d1', 'Dune', 1), book('f1', 'Foundation', 1),
            ].map(b => b.id.startsWith('f') ? { ...b, metadata: { ...b.metadata, series: 'Foundation' } } : b)
            open(resolvingSubmit(), mixed)
            expect(listed()).toEqual(['Dune', 'Foundation', 'Foundation and Empire', 'An Extra'])
        })

        it('lists the books beside the form, by volume then title', () => {
            open(resolvingSubmit(), trilogy)
            expect(document.querySelector('.metadata-books')?.getAttribute('aria-label')).toBe('Selected books')
            expect(listed()).toEqual(['Dune', 'Dune Messiah', 'Children of Dune', 'An Extra'])
            expect(numbers()).toEqual(['1', '2', '3', '4'])
        })

        it('offers to number the volumes only once a series is filled in', async () => {
            const w = open(resolvingSubmit(), trilogy)
            expect(numbering().disabled).toBe(true)
            type(fieldLabelled('Series'), 'Dune')
            await w.vm.$nextTick()
            expect(numbering().disabled).toBe(false)
        })

        it('numbers the volumes in the order of the list, from the first volume chosen', async () => {
            const submit = resolvingSubmit()
            const w = open(submit, trilogy)
            type(fieldLabelled('Series'), 'Dune')
            await w.vm.$nextTick()
            numbering().click()
            await w.vm.$nextTick()
            expect(numbers()).toEqual(['Vol. 1', 'Vol. 2', 'Vol. 3', 'Vol. 4'])

            // "An Extra" goes first, from the keyboard; the series starts at 0 (a prequel).
            const handles = () => Array.from(document.querySelectorAll<HTMLButtonElement>('.batch-order__handle'))
            keydown(handles()[3], 'Home')
            await w.vm.$nextTick()
            expect(listed()).toEqual(['An Extra', 'Dune', 'Dune Messiah', 'Children of Dune'])
            expect(document.activeElement).toBe(handles()[0])
            expect(document.querySelector('.batch-order [role="status"]')?.textContent).toBe('“An Extra” is now number 1 of 4.')
            type(fieldLabelled('First volume'), '0')
            await w.vm.$nextTick()
            expect(numbers()).toEqual(['Vol. 0', 'Vol. 1', 'Vol. 2', 'Vol. 3'])

            form().dispatchEvent(new Event('submit'))
            await flushPromises()
            expect(submit).toHaveBeenCalledWith(
                ['x', 'a', 'b', 'c'],
                { series: 'Dune' },
                { action: 'none', base64: null },
                { x: 0, a: 1, b: 2, c: 3 },
            )
        })

        it('refuses a first volume that isn\'t a number', async () => {
            const submit = resolvingSubmit()
            const w = open(submit, trilogy)
            type(fieldLabelled('Series'), 'Dune')
            await w.vm.$nextTick()
            numbering().click()
            await w.vm.$nextTick()
            type(fieldLabelled('First volume'), 'one')
            await w.vm.$nextTick()
            form().dispatchEvent(new Event('submit'))
            await flushPromises()
            expect(submit).not.toHaveBeenCalled()
        })
    })

    it('stays open and shows the error when the save fails', async () => {
        const submit = vi.fn(() => Promise.reject(new Error('2 of 2 books could not be updated: read-only')))
        const w = open(submit)
        form().dispatchEvent(new Event('submit'))
        await flushPromises()

        expect(w.emitted('close')).toBeUndefined()
        expect(top().querySelector('[role="alert"]')?.textContent).toContain('read-only')
    })

    it('is a labelled dialog whose every field has a linked label, with focus on the first one', () => {
        open(resolvingSubmit(), [books[0]])
        const dialog = top()
        expect(document.getElementById(dialog.getAttribute('aria-labelledby')!)?.textContent).toContain('Edit metadata')
        const controls = Array.from(form().querySelectorAll<HTMLElement>('input:not([type="checkbox"]), textarea'))
        expect(controls).toHaveLength(7)
        for (const control of controls) {
            expect(document.querySelector(`label[for="${control.id}"]`), control.outerHTML).not.toBeNull()
        }
        // Categories: a fieldset with a legend, each checkbox inside its label.
        expect(form().querySelector('fieldset legend')?.textContent).toContain('Categories')
        expect(document.activeElement).toBe(fieldLabelled('Title'))
    })

    it('closes on Escape when nothing changed, and gives focus back', () => {
        const w = open(resolvingSubmit(), [books[0]])
        keydown(top(), 'Escape')
        expect(w.emitted('close')).toHaveLength(1)
        w.unmount()
        expect(document.activeElement).toBe(opener)
    })

    it('asks before discarding edits on Escape, and keeps them on "Keep editing"', async () => {
        const w = open(resolvingSubmit(), [books[0]])
        type(fieldLabelled('Author'), 'Someone else')
        await flushPromises()
        keydown(top(), 'Escape')
        await flushPromises()
        expect(w.emitted('close')).toBeUndefined()
        expect(dialogs()).toHaveLength(2)
        expect(top().textContent).toContain('Discard your changes?')

        Array.from(top().querySelectorAll('button')).find(b => b.textContent?.includes('Keep editing'))!.click()
        await flushPromises()
        expect(dialogs()).toHaveLength(1)
        expect(fieldLabelled('Author').value).toBe('Someone else')

        keydown(top(), 'Escape')
        await flushPromises()
        Array.from(top().querySelectorAll('button')).find(b => b.textContent?.trim() === 'Discard')!.click()
        await flushPromises()
        expect(w.emitted('close')).toHaveLength(1)
    })

    it('writes the whole form for one book: a cleared series is removed, "2,5" is volume 2.5', async () => {
        const submit = resolvingSubmit()
        open(submit, [books[0]])
        type(fieldLabelled('Volume'), '2,5')
        form().dispatchEvent(new Event('submit'))
        await flushPromises()
        expect(submit.mock.calls[0][1]).toMatchObject({ title: 'Title a', series: 'Saga', seriesIndex: 2.5 })

        cleanup()
        const submit2 = resolvingSubmit()
        open(submit2, [books[0]])
        type(fieldLabelled('Series'), '')
        type(fieldLabelled('Volume'), '')
        form().dispatchEvent(new Event('submit'))
        await flushPromises()
        expect(submit2.mock.calls[0][1]).toMatchObject({ series: undefined, seriesIndex: undefined })
    })

    it('refuses a volume that isn’t a number, and says why on the field', async () => {
        const submit = resolvingSubmit()
        open(submit, [books[0]])
        const volume = fieldLabelled('Volume')
        type(volume, 'two')
        await flushPromises()
        form().dispatchEvent(new Event('submit'))
        await flushPromises()
        expect(submit).not.toHaveBeenCalled()
        expect(volume.getAttribute('aria-invalid')).toBe('true')
        expect(document.getElementById(volume.getAttribute('aria-describedby')!.split(' ').pop()!)?.textContent).toContain('Enter a number')
    })

    it('removes the cover from a button (reachable from the keyboard), and that counts as an edit', async () => {
        useLibraryStore(testPinia()).setCover('a', { coverPath: '/lib/.covers/a.jpg', coverVersion: 1 })
        const submit = resolvingSubmit()
        const w = open(submit, [books[0]])
        await flushPromises()
        expect(top().querySelector<HTMLImageElement>('.cover-img')?.alt).toBe('Cover')
        const remove = Array.from(top().querySelectorAll('button')).find(b => b.textContent?.includes('Remove cover'))!
        remove.click()
        await flushPromises()
        expect(top().textContent).toContain('The cover will be removed when you save.')
        keydown(top(), 'Escape')
        await flushPromises()
        expect(w.emitted('close')).toBeUndefined()   // the discard guard asks first
        Array.from(top().querySelectorAll('button')).find(b => b.textContent?.includes('Keep editing'))!.click()
        await flushPromises()

        form().dispatchEvent(new Event('submit'))
        await flushPromises()
        expect(submit.mock.calls[0][2]).toEqual({ action: 'delete', base64: null })
    })

    it('can’t be closed while saving', async () => {
        const saving = deferred()
        const w = open(() => saving.promise, [books[0]])
        form().dispatchEvent(new Event('submit'))
        await flushPromises()
        const save = Array.from(top().querySelectorAll('button')).find(b => b.getAttribute('type') === 'submit')!
        expect(save.getAttribute('aria-busy')).toBe('true')
        keydown(top(), 'Escape')
        expect(w.emitted('close')).toBeUndefined()
        saving.resolve()
        await flushPromises()
        expect(w.emitted('close')).toHaveLength(1)
    })
})
