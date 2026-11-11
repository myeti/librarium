import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { defineComponent, h } from 'vue'

vi.mock('@tauri-apps/api/core', () => import('@/test/tauri'))
vi.mock('@/services/librarium-api', () => ({ reportError: vi.fn() }))
import { backendError, mockInvoke } from '@/test/tauri'
import DuplicatesModal from './DuplicatesModal.vue'
import { useLibraryStore } from '@/stores/library'
import { useNotificationsStore } from '@/stores/notifications'
import { cleanup, keydown, mountUi, testPinia } from '@/test/mountUi'
import type { Book, BookMetadata } from '@/types'

function book(id: string, metadata: Partial<BookMetadata> = {}, extra: Partial<Book> = {}): Book {
    return {
        id, path: `/lib/${id}.epub`, filename: `${id}.epub`, format: 'epub', sizeBytes: 2_500_000, dateAdded: '2026-01-01T00:00:00Z',
        metadata: { title: id.replace(/\d$/, ''), author: 'A', categories: [], ...metadata },
        ...extra,
    }
}

const LIBRARY: Book[] = [
    book('dune1'),
    // More complete: the suggested copy of its group.
    book('dune2', { isbn: '9780441013593', description: 'Desert planet.' }),
    book('emma1', { isbn: '9780141439587' }),
    book('emma2', { isbn: '9780141439587' }),
    book('emma3', { isbn: '9780141439587' }),
]

/** What LibraryView does: the dialog gets the store's live duplicate groups. */
const Host = defineComponent({
    emits: ['close'],
    render() {
        return h(DuplicatesModal, { groups: useLibraryStore().duplicateGroups, onClose: () => this.$emit('close') })
    },
})

const dialogs = () => Array.from(document.querySelectorAll('dialog'))
const top = () => dialogs()[dialogs().length - 1]
const groups = () => Array.from(document.querySelectorAll<HTMLElement>('.dup-group'))
/** The group of a book (ISBN groups come first: Emma, then Dune). */
const groupOf = (id: string) => groups().find(g => g.querySelector(`.dup-path[title="/lib/${id}.epub"]`))!
const rowOf = (id: string) => Array.from(document.querySelectorAll<HTMLElement>('.dup-book'))
    .find(r => r.querySelector('.dup-path')?.getAttribute('title') === `/lib/${id}.epub`)!
const button = (cls: string) => document.querySelector<HTMLButtonElement>(`.${cls}`)!

describe('DuplicatesModal', () => {
    beforeEach(async () => {
        mockInvoke.reset()
        mockInvoke.on('scan_library', () => LIBRARY.map(b => ({ ...b })))
        mockInvoke.on('ensure_cover', () => null)
        mockInvoke.on('invalidate_opds_cache', () => null)
        await useLibraryStore(testPinia()).openLibrary('/lib')
    })

    afterEach(cleanup)

    async function open() {
        const host = mountUi(Host)
        await flushPromises()
        return host
    }

    it('opens labelled, with the suggested copy of each group chosen and the reason said', async () => {
        await open()
        const dialog = top()
        expect(document.getElementById(dialog.getAttribute('aria-labelledby')!)?.textContent).toContain('Duplicates')
        expect(groups()).toHaveLength(2)

        const dune2 = rowOf('dune2')
        expect(dune2.querySelector<HTMLInputElement>('input')!.checked).toBe(true)
        expect(dune2.textContent).toContain('Suggested')
        expect(dune2.textContent).toContain('most complete details')
        expect(dune2.textContent).toContain('Keep')
        // What tells copies apart: format, size, date added, completeness, what's missing.
        const dune1 = rowOf('dune1')
        expect(dune1.textContent).toContain('EPUB')
        expect(dune1.textContent).toContain('2.4 MB')
        expect(dune1.textContent).toMatch(/Added .*2026/)
        expect(dune1.textContent).toContain('1/7 details')
        expect(dune1.textContent).toContain('Missing: Series, Language, ISBN, Description, Categories, Cover')
        expect(dune1.textContent).toContain('Extra copy')

        // Focus starts on a choice, inside the dialog.
        expect((document.activeElement as HTMLInputElement).classList.contains('dup-radio')).toBe(true)
        expect(button('dup-trash').textContent).toContain('Move 3 copies to Trash')
        expect(button('dup-forget').textContent).toContain('Remove 3 copies from library')
    })

    it('lets a group be skipped: nothing happens to its copies', async () => {
        await open()
        const emma = groupOf('emma1')
        emma.querySelector<HTMLInputElement>('.dup-book.-skip input')!.click()
        await flushPromises()
        expect(emma.textContent).toContain('Keeping all copies')
        expect(emma.textContent).not.toContain('Extra copy')
        expect(button('dup-trash').textContent).toContain('Move 1 copy to Trash')
        expect(document.querySelector('.dup-summary')!.textContent).toBe('1 extra copy selected · 1 group skipped')

        groupOf('dune1').querySelector<HTMLInputElement>('.dup-book.-skip input')!.click()
        await flushPromises()
        expect(button('dup-trash').disabled).toBe(true)
        expect(button('dup-forget').disabled).toBe(true)
    })

    it('moves the extra copies to the Trash after a confirmation on top; Escape closes only that one', async () => {
        mockInvoke.on('delete_books', ({ bookIds }) => ({ deleted: bookIds, failures: [], libraryTrash: null }))
        const host = await open()
        rowOf('dune1').querySelector('input')!.click()          // another copy than the suggested one
        await flushPromises()

        button('dup-trash').click()
        await flushPromises()
        expect(dialogs()).toHaveLength(2)
        expect(top().textContent).toContain('Move 3 extra copies to the Trash?')
        keydown(top(), 'Escape')
        await flushPromises()
        expect(dialogs()).toHaveLength(1)
        expect(host.emitted('close')).toBeUndefined()

        button('dup-trash').click()
        await flushPromises()
        Array.from(top().querySelectorAll('button')).find(b => b.textContent?.includes('Move to Trash'))!.click()
        await flushPromises()

        expect(mockInvoke.callsTo('delete_books')[0].args.bookIds).toEqual(['emma2', 'emma3', 'dune2'])
        expect(useLibraryStore().books.map(b => b.id)).toEqual(['dune1', 'emma1'])
        expect(useNotificationsStore().items.map(n => n.message)).toEqual(['Moved 3 books to the Trash.'])
        // No group left: the review is over.
        expect(host.emitted('close')).toHaveLength(1)
    })

    it('removes them from the library instead, keeping the files', async () => {
        mockInvoke.on('forget_books', ({ bookIds }) => ({ forgotten: bookIds, failures: [] }))
        await open()
        groupOf('emma1').querySelector<HTMLInputElement>('.dup-book.-skip input')!.click()
        await flushPromises()
        button('dup-forget').click()
        await flushPromises()
        expect(dialogs()).toHaveLength(2)
        expect(top().textContent).toContain('Remove 1 extra copy from the library?')
        Array.from(top().querySelectorAll('button')).find(b => b.textContent?.includes('Remove from library'))!.click()
        await flushPromises()
        expect(mockInvoke.callsTo('forget_books')[0].args.bookIds).toEqual(['dune1'])
        expect(mockInvoke.callsTo('delete_books')).toHaveLength(0)
        // The skipped group is still there, still skipped, and focus stays in the dialog.
        expect(groups()).toHaveLength(1)
        expect(groups()[0].textContent).toContain('Keeping all copies')
        expect(top().contains(document.activeElement)).toBe(true)
    })

    it('keeps the confirmation open with the reason when nothing could be done', async () => {
        mockInvoke.on('delete_books', () => backendError({ code: 'IO', message: 'The NAS is offline' }))
        const host = await open()
        button('dup-trash').click()
        await flushPromises()
        Array.from(top().querySelectorAll('button')).find(b => b.textContent?.includes('Move to Trash'))!.click()
        await flushPromises()
        expect(dialogs()).toHaveLength(2)
        expect(top().querySelector('[role="alert"]')!.textContent).toContain('The NAS is offline')
        expect(host.emitted('close')).toBeUndefined()
        expect(useLibraryStore().books).toHaveLength(5)
    })

    it('says which copies could not be removed, and keeps their groups', async () => {
        mockInvoke.on('delete_books', () => ({ deleted: ['dune1'], failures: [{ bookId: 'emma2', error: 'file is locked' }, { bookId: 'emma3', error: 'file is locked' }], libraryTrash: null }))
        const host = await open()
        button('dup-trash').click()
        await flushPromises()
        Array.from(top().querySelectorAll('button')).find(b => b.textContent?.includes('Move to Trash'))!.click()
        await flushPromises()
        expect(dialogs()).toHaveLength(1)
        expect(document.querySelector('.dup-notice')!.textContent).toContain('2 books could not be moved to the Trash: file is locked')
        expect(groups()).toHaveLength(1)
        expect(host.emitted('close')).toBeUndefined()
    })

    it('keeps each choice with its book when the groups change, and drops one whose book left', async () => {
        await open()
        rowOf('dune1').querySelector('input')!.click()
        rowOf('emma3').querySelector('input')!.click()
        await flushPromises()
        const library = useLibraryStore()
        // The library changed meanwhile: the groups come back in another order…
        library.books = [...library.books].reverse()
        await flushPromises()
        expect(rowOf('dune1').querySelector('input')!.checked).toBe(true)
        expect(rowOf('emma3').querySelector('input')!.checked).toBe(true)
        // …and the copy chosen in one of them is gone: that group is skipped, not re-decided.
        library.removeBooks(['emma3'])
        await flushPromises()
        expect(groupOf('emma1').textContent).toContain('Keeping all copies')
        expect(button('dup-trash').textContent).toContain('Move 1 copy to Trash')
    })

    it('starts a group that appears after opening as skipped', async () => {
        await open()
        useLibraryStore().addBooks([book('neuromancer1'), book('neuromancer2')])
        await flushPromises()
        expect(groups()).toHaveLength(3)
        expect(groupOf('neuromancer1').textContent).toContain('Keeping all copies')
        expect(button('dup-trash').textContent).toContain('Move 3 copies to Trash')
    })

    it('names the reason of each group', async () => {
        await open()
        expect(Array.from(document.querySelectorAll('.dup-reason')).map(r => r.textContent)).toEqual([
            'Same ISBN: 9780141439587', 'Same title and author',
        ])
    })
})
