import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'

vi.mock('@tauri-apps/api/core', () => import('@/test/tauri'))
vi.mock('@/utils/sounds', () => ({ playNewUser: vi.fn(), playIntro: vi.fn(), playTransferComplete: vi.fn() }))
vi.mock('@tauri-apps/api/path', () => ({
    documentDir: () => Promise.resolve('/Users/me/Documents'),
    join: (...parts: string[]) => Promise.resolve(parts.join('/')),
}))
type Handler = (event: { payload: unknown }) => void
const listeners = new Map<string, Handler>()
vi.mock('@tauri-apps/api/event', () => ({
    listen: vi.fn((name: string, handler: Handler) => {
        listeners.set(name, handler)
        return Promise.resolve(() => { listeners.delete(name) })
    }),
}))
import { backendError, mockInvoke } from '@/test/tauri'
import { useBackupStore } from './backup'
import { useImportsStore } from './imports'
import { useLibraryStore } from './library'
import { useNotificationsStore } from './notifications'
import { useUsersStore } from './users'
import type { BackupExported, BackupManifest, BackupProgress, BackupRestored } from '@/types'

const ROOT = '/Users/me/Documents/Librarium'

const MANIFEST: BackupManifest = {
    app: 'Librarium', format: 1, appVersion: '0.6.0', createdAt: '2026-11-10T13:32:00Z',
    users: ['ann', 'bob'], files: 10, bytes: 1000,
}

/** A promise the test resolves by hand: the backend at work. */
function deferred<T>() {
    let resolve!: (v: T) => void
    let reject!: (e: unknown) => void
    const promise = new Promise<T>((res, rej) => { resolve = res; reject = rej })
    return { promise, resolve, reject }
}

function emitProgress(progress: BackupProgress) {
    listeners.get('backup-progress')!({ payload: progress })
}

/** Lets the awaits before the backend call run. */
const settle = () => new Promise(resolve => setTimeout(resolve, 0))

/** Signed in as `user` among `users`, the library open. */
async function signedIn(user: string, users: string[]) {
    mockInvoke.on('list_users', () => users)
    const store = useUsersStore()
    store.libraryRoot = ROOT
    store.users = [...users]
    await store.selectUser(user)
    return store
}

function successToasts(): string[] {
    return useNotificationsStore().items.filter(n => n.type === 'success').map(n => n.message)
}

beforeEach(() => {
    setActivePinia(createPinia())
    mockInvoke.reset()
    listeners.clear()
    localStorage.clear()
    mockInvoke.on('ensure_cover', () => null)
    mockInvoke.on('library_lock_status', () => ({ otherInstance: null }))
    mockInvoke.on('scan_library', () => [])
})

describe('backup store: export', () => {
    it('backs up the library root into the chosen folder, following the progress', async () => {
        await signedIn('ann', ['ann'])
        const backup = useBackupStore()
        const running = deferred<BackupExported>()
        mockInvoke.on('export_backup', () => running.promise)

        const done = backup.exportTo('/Backups')
        await settle()
        expect(mockInvoke.callsTo('export_backup')[0].args).toEqual({ libraryRoot: ROOT, destDir: '/Backups' })
        expect(backup.running).toBe('export')
        expect(backup.percent).toBeNull()
        emitProgress({ phase: 'copying', filesDone: 3, filesTotal: 10, bytesDone: 250, bytesTotal: 1000 })
        expect(backup.percent).toBe(25)

        running.resolve({ path: '/Backups/Librarium backup 2026-11-10 14.32.zip', users: ['ann'], files: 10, bytes: 1000, cancelled: false })
        await done
        expect(backup.running).toBeNull()
        expect(backup.progress).toBeNull()
        expect(listeners.has('backup-progress')).toBe(false)
        const toast = useNotificationsStore().items.find(n => n.type === 'success')!
        expect(toast.message).toContain('Librarium backup 2026-11-10 14.32.zip')
        // Its button shows the file (the backend reveals the last backup only).
        mockInvoke.on('reveal_backup', () => undefined)
        toast.action!.run()
        expect(mockInvoke.callsTo('reveal_backup')).toHaveLength(1)
    })

    it('says nothing when cancelled, and refuses a second operation meanwhile', async () => {
        await signedIn('ann', ['ann'])
        const backup = useBackupStore()
        const running = deferred<BackupExported>()
        mockInvoke.on('export_backup', () => running.promise)
        mockInvoke.on('cancel_backup', () => true)

        const done = backup.exportTo('/Backups')
        await settle()
        await expect(backup.exportTo('/Elsewhere')).rejects.toThrow(/^CONFLICT/)
        await expect(backup.restoreFrom('/b.zip', MANIFEST)).rejects.toThrow(/^CONFLICT/)
        expect(useLibraryStore().libraryPath).toBe(`${ROOT}/ann`)

        await backup.cancel()
        expect(backup.cancelling).toBe(true)
        expect(mockInvoke.callsTo('cancel_backup')).toHaveLength(1)
        running.resolve({ path: null, users: ['ann'], files: 10, bytes: 1000, cancelled: true })
        expect((await done).cancelled).toBe(true)
        expect(successToasts()).toEqual([])
        expect(backup.cancelling).toBe(false)
    })
})

describe('backup store: restore', () => {
    it('closes the library while restoring, then starts afresh: several users, the picker', async () => {
        const users = await signedIn('ann', ['ann'])
        const library = useLibraryStore()
        const backup = useBackupStore()
        const running = deferred<BackupRestored>()
        mockInvoke.on('restore_backup', () => running.promise)

        const done = backup.restoreFrom('/Backups/b.zip', MANIFEST)
        await settle()
        expect(mockInvoke.callsTo('restore_backup')[0].args).toEqual({ libraryRoot: ROOT, path: '/Backups/b.zip' })
        // Closed, but still signed in: the picker doesn't replace the screen mid-restore.
        expect(library.libraryPath).toBeNull()
        expect(users.currentUser).toBe('ann')

        mockInvoke.on('list_users', () => ['ann', 'bob'])
        running.resolve({ users: ['ann', 'bob'], files: 10, cancelled: false })
        await done
        expect(users.users).toEqual(['ann', 'bob'])
        expect(users.currentUser).toBeNull()
        expect(successToasts()).toHaveLength(1)
        expect(successToasts()[0]).toContain('2')
    })

    it('reopens the user’s library after restoring when they are the only user', async () => {
        const users = await signedIn('ann', ['ann'])
        mockInvoke.on('restore_backup', () => ({ users: ['ann'], files: 3, cancelled: false }))
        await useBackupStore().restoreFrom('/b.zip', { ...MANIFEST, users: ['ann'] })
        expect(users.currentUser).toBe('ann')
        expect(useLibraryStore().libraryPath).toBe(`${ROOT}/ann`)
    })

    it('reopens the library as it was when cancelled or when it fails', async () => {
        const users = await signedIn('ann', ['ann', 'bob'])
        const backup = useBackupStore()
        mockInvoke.on('restore_backup', () => ({ users: [], files: 0, cancelled: true }))
        expect((await backup.restoreFrom('/b.zip', MANIFEST)).cancelled).toBe(true)
        expect(users.currentUser).toBe('ann')
        expect(useLibraryStore().libraryPath).toBe(`${ROOT}/ann`)

        mockInvoke.on('restore_backup', () => backendError({ code: 'IO', message: 'Not enough space for the restore' }))
        await expect(backup.restoreFrom('/b.zip', MANIFEST)).rejects.toMatchObject({ code: 'IO' })
        expect(users.currentUser).toBe('ann')
        expect(useLibraryStore().libraryPath).toBe(`${ROOT}/ann`)
        expect(successToasts()).toEqual([])
        expect(backup.running).toBeNull()
    })

    it('waits for imports to end: their books would be erased', async () => {
        await signedIn('ann', ['ann'])
        const backup = useBackupStore()
        useImportsStore().pending = 1
        expect(backup.restoreBlocker).toBeTruthy()
        await expect(backup.restoreFrom('/b.zip', MANIFEST)).rejects.toThrow(/^CONFLICT/)
        expect(mockInvoke.callsTo('restore_backup')).toHaveLength(0)
        expect(useLibraryStore().libraryPath).toBe(`${ROOT}/ann`)
    })
})
