import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'

vi.mock('@tauri-apps/api/core', () => import('@/test/tauri'))
vi.mock('@/utils/sounds', () => ({ playNewUser: vi.fn(), playIntro: vi.fn(), playTransferComplete: vi.fn() }))
vi.mock('@tauri-apps/api/path', () => ({
    documentDir: () => Promise.resolve('/Users/me/Documents'),
    join: (...parts: string[]) => Promise.resolve(parts.join('/')),
}))
import { backendError, mockInvoke } from '@/test/tauri'
import { useUsersStore } from './users'
import { useLibraryStore } from './library'
import { useSettingsStore } from './settings'

/** What the backend rejects with when the library root isn't there (an AppError). */
const UNAVAILABLE = { code: 'LIBRARY_UNAVAILABLE', message: 'The library folder /Volumes/books can’t be reached. Is the drive or NAS connected?' }

beforeEach(() => {
    setActivePinia(createPinia())
    mockInvoke.reset()
    localStorage.clear()
    mockInvoke.on('ensure_cover', () => null)
    mockInvoke.on('scan_library', ({ path }) => [{
        id: `${String(path)}-1`, path: `${String(path)}/a.epub`, filename: 'a.epub', format: 'epub', sizeBytes: 1,
        dateAdded: '2026-01-01', metadata: { title: 'A', author: 'B', categories: [] },
    }])
})

describe('users store owns the open library', () => {
    it('opens the selected user’s library, whatever view is on screen', async () => {
        const users = useUsersStore()
        users.libraryRoot = '/root'
        await users.selectUser('ann')
        expect(useLibraryStore().libraryPath).toBe('/root/ann')
        expect(mockInvoke.callsTo('scan_library').map(c => c.args.path)).toEqual(['/root/ann'])
    })

    it('switches the library when the user changes, and closes it on the picker', async () => {
        const users = useUsersStore()
        const library = useLibraryStore()
        users.libraryRoot = '/root'
        await users.selectUser('ann')
        library.toggleSelect('/root/ann-1')

        await users.selectUser('bob')
        expect(library.libraryPath).toBe('/root/bob')
        expect(library.books.map(b => b.id)).toEqual(['/root/bob-1'])
        expect(library.selectedIds.size).toBe(0)

        await users.clearUser()
        expect(library.libraryPath).toBeNull()
        expect(library.books).toEqual([])
    })
})

describe('users store: a library root that is not there', () => {
    it('reports a chosen root that is unreachable, and keeps the current user', async () => {
        useSettingsStore().libraryRootOverride = '/Volumes/books'
        localStorage.setItem('librarium:currentUser', 'ann')
        mockInvoke.on('list_users', () => backendError(UNAVAILABLE))
        mockInvoke.on('scan_library', () => backendError({ ...UNAVAILABLE, message: UNAVAILABLE.message.replace('/Volumes/books', '/Volumes/books/ann') }))
        const users = useUsersStore()
        await users.init()
        expect(users.unavailable).toBe(true)
        expect(users.errorCode).toBe('LIBRARY_UNAVAILABLE')
        expect(users.error).toBe(UNAVAILABLE.message)
        expect(users.currentUser).toBe('ann')
        await Promise.resolve()
        expect(mockInvoke.callsTo('scan_library').map(c => c.args.path)).toEqual(['/Volumes/books/ann'])

        // Mounted again: Retry reads the users and reopens the library.
        mockInvoke.on('list_users', () => ['ann'])
        mockInvoke.on('scan_library', () => [])
        await users.retry()
        expect(users.unavailable).toBe(false)
        expect(users.error).toBeNull()
        expect(users.errorCode).toBeNull()
        expect(useLibraryStore().unavailable).toBe(false)
    })

    /**
     * macOS refusing access to /Volumes (privacy settings), an expired SMB
     * session, an I/O error: the users are unknown, not "none". Never treated
     * as "no users", which would forget the remembered user (logging them
     * out) and offer only "Add user", with no Retry.
     */
    it.each([
        ['a permission refusal', { code: 'PERMISSION_DENIED', message: 'Librarium isn’t allowed to open the library folder "books". In System Settings › Privacy & Security › Files and Folders…' }],
        ['an I/O error', { code: 'IO', message: 'Could not read the folder books: Input/output error' }],
        ['a plain string', 'Something odd happened'],
    ])('keeps the current user after %s, and reports it', async (_, failure) => {
        useSettingsStore().libraryRootOverride = '/Volumes/books'
        localStorage.setItem('librarium:currentUser', 'ann')
        mockInvoke.on('list_users', () => backendError(failure))
        const users = useUsersStore()
        await users.init()

        expect(users.unavailable).toBe(true)
        expect(users.currentUser).toBe('ann')
        expect(localStorage.getItem('librarium:currentUser')).toBe('ann')
        expect(users.error).toBe(typeof failure === 'string' ? failure : failure.message)
        expect(users.errorCode).toBe(typeof failure === 'string' ? null : failure.code)
        // The library is still opened for the remembered user (its view reports its own state).
        await Promise.resolve()
        expect(mockInvoke.callsTo('scan_library').map(c => c.args.path)).toEqual(['/Volumes/books/ann'])

        // Allowed (or reconnected): Retry lists the users again.
        mockInvoke.on('list_users', () => ['ann', 'bob'])
        await users.retry()
        expect(users.unavailable).toBe(false)
        expect(users.users).toEqual(['ann', 'bob'])
        expect(users.currentUser).toBe('ann')
    })

    it('treats even the default root as unknown when it can’t be read', async () => {
        // Missing default root = first run; a refused one is not.
        mockInvoke.on('list_users', () => backendError({ code: 'PERMISSION_DENIED', message: 'not allowed' }))
        const users = useUsersStore()
        await users.init()
        expect(users.unavailable).toBe(true)
        expect(users.errorCode).toBe('PERMISSION_DENIED')
    })

    it('treats a missing default root as a first run', async () => {
        mockInvoke.on('list_users', () => backendError(UNAVAILABLE))
        const users = useUsersStore()
        await users.init()
        expect(users.libraryRoot).toBe('/Users/me/Documents/Librarium')
        expect(users.unavailable).toBe(false)
        expect(users.users).toEqual([])

        // Creating the first user may create the default folder…
        mockInvoke.on('create_user', ({ name }) => name)
        await users.createUser('Ann')
        expect(mockInvoke.callsTo('create_user')[0].args).toEqual({
            libraryPath: '/Users/me/Documents/Librarium', name: 'Ann', createRoot: true,
        })
    })

    it('never asks to create a chosen root', async () => {
        useSettingsStore().libraryRootOverride = '/Volumes/books'
        const users = useUsersStore()
        users.libraryRoot = '/Volumes/books'
        mockInvoke.on('create_user', () => backendError(UNAVAILABLE))
        expect(await users.createUser('Ann')).toBeNull()
        expect(mockInvoke.callsTo('create_user')[0].args.createRoot).toBe(false)
        // Shown without the machine-readable code.
        expect(users.error).toBe(UNAVAILABLE.message)
    })
})
