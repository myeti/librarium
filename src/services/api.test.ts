import { beforeEach, describe, expect, it, vi } from 'vitest'
import { reactive } from 'vue'

vi.mock('@tauri-apps/api/core', () => import('@/test/tauri'))
const eventListeners = new Map<string, (event: { payload: unknown }) => void>()
vi.mock('@tauri-apps/api/event', () => ({
    listen: vi.fn((name: string, handler: (event: { payload: unknown }) => void) => {
        eventListeners.set(name, handler)
        return Promise.resolve(() => { eventListeners.delete(name) })
    }),
}))
import { backendError, mockInvoke } from '@/test/tauri'
import * as api from './api'

/**
 * Each wrapper must call its command with exactly the arguments docs/IPC.md
 * lists (camelCase), and pass the result and the rejection through as they are.
 */
const CALLS: [string, () => Promise<unknown>, string, Record<string, unknown>][] = [
    ['scanLibrary', () => api.scanLibrary('/lib'), 'scan_library', { path: '/lib' }],
    ['importBooks', () => api.importBooks(['/a.epub'], '/lib'), 'import_books', { paths: ['/a.epub'], libraryPath: '/lib' }],
    ['forgetBooks', () => api.forgetBooks('/lib', ['a']), 'forget_books', { libraryPath: '/lib', bookIds: ['a'] }],
    ['libraryLockStatus', () => api.libraryLockStatus('/lib'), 'library_lock_status', { libraryPath: '/lib' }],
    ['updateMetadata', () => api.updateMetadata('/lib', 'a', { title: 'T', author: 'A', categories: [] }), 'update_metadata',
        { libraryPath: '/lib', bookId: 'a', metadata: { title: 'T', author: 'A', categories: [] } }],
    ['batchUpdateMetadata', () => api.batchUpdateMetadata('/lib', ['a', 'b'], { author: 'A' }), 'batch_update_metadata',
        { libraryPath: '/lib', bookIds: ['a', 'b'], patch: { author: 'A' } }],
    ['batchUpdateMetadata (numbered)', () => api.batchUpdateMetadata('/lib', ['a', 'b'], { series: 'S' }, { a: 2, b: 1 }), 'batch_update_metadata',
        { libraryPath: '/lib', bookIds: ['a', 'b'], patch: { series: 'S' }, seriesIndexes: { a: 2, b: 1 } }],
    ['renameBook', () => api.renameBook('/lib', 'a', 'x.epub'), 'rename_book', { libraryPath: '/lib', bookId: 'a', newFilename: 'x.epub' }],
    ['deleteBooks', () => api.deleteBooks('/lib', ['a']), 'delete_books', { libraryPath: '/lib', bookIds: ['a'] }],
    ['resetBookData', () => api.resetBookData('/lib', 'a'), 'reset_book_data', { libraryPath: '/lib', bookId: 'a' }],
    ['convertToEpub', () => api.convertToEpub('/lib', 'a'), 'convert_to_epub', { libraryPath: '/lib', bookId: 'a' }],
    ['ensureCover', () => api.ensureCover('/lib', 'a'), 'ensure_cover', { libraryPath: '/lib', bookId: 'a' }],
    ['saveCoverFromUrl', () => api.saveCoverFromUrl('/lib', 'a', 'https://x/c.jpg'), 'save_cover_from_url',
        { libraryPath: '/lib', bookId: 'a', url: 'https://x/c.jpg' }],
    ['setBookCover', () => api.setBookCover('/lib', 'a', 'AAAA'), 'set_book_cover', { libraryPath: '/lib', bookId: 'a', imageBase64: 'AAAA' }],
    ['deleteBookCover', () => api.deleteBookCover('/lib', 'a'), 'delete_book_cover', { libraryPath: '/lib', bookId: 'a' }],
    ['downloadAndImport', () => api.downloadAndImport('https://x/b.epub', 'b.epub', '/lib'), 'download_and_import',
        { url: 'https://x/b.epub', filename: 'b.epub', libraryPath: '/lib' }],
    ['exportBook', () => api.exportBook('/lib', 'a', 'azw3', '/out/a.azw3'), 'export_book',
        { libraryPath: '/lib', bookId: 'a', targetFormat: 'azw3', destPath: '/out/a.azw3' }],
    ['inspectEpub', () => api.inspectEpub('/lib/a.epub'), 'inspect_epub', { path: '/lib/a.epub' }],
    ['detectLanguage', () => api.detectLanguage('Dune Frank Herbert'), 'detect_language', { text: 'Dune Frank Herbert' }],
    // Reader and Calibre import
    ['openForReading', () => api.openForReading('/lib', 'a'), 'open_for_reading', { libraryPath: '/lib', bookId: 'a' }],
    ['detectCalibreLibrary', () => api.detectCalibreLibrary(), 'detect_calibre_library', {}],
    ['listCalibreBooks', () => api.listCalibreBooks('/calibre'), 'list_calibre_books', { calibrePath: '/calibre' }],
    ['importFromCalibre', () => api.importFromCalibre('/calibre', [1, 2], '/lib'), 'import_from_calibre',
        { calibrePath: '/calibre', bookIds: [1, 2], libraryPath: '/lib' }],
    ['cancelCalibreImport', () => api.cancelCalibreImport(), 'cancel_calibre_import', {}],
    ['listUsers', () => api.listUsers('/root'), 'list_users', { libraryPath: '/root' }],
    ['createUser', () => api.createUser('/root', 'Ann', false), 'create_user', { libraryPath: '/root', name: 'Ann', createRoot: false }],
    ['revealInExplorer', () => api.revealInExplorer('/lib', '/lib/a.epub'), 'reveal_in_explorer', { libraryPath: '/lib', path: '/lib/a.epub' }],
    ['checkPathAccessible', () => api.checkPathAccessible('/nas'), 'check_path_accessible', { path: '/nas' }],
    ['openLogFolder', () => api.openLogFolder(), 'open_log_folder', {}],
    ['getConfig', () => api.getConfig(), 'get_config', {}],
    ['setConfig', () => api.setConfig({ lang: 'fr' }), 'set_config', { updates: { lang: 'fr' } }],
    ['detectDevices', () => api.detectDevices(), 'detect_devices', {}],
    ['detectDevice', () => api.detectDevice(), 'detect_device', {}],
    ['ejectDevice', () => api.ejectDevice('/Volumes/Kindle'), 'eject_device', { mountPath: '/Volumes/Kindle' }],
    ['scanDeviceLibrary', () => api.scanDeviceLibrary('/Volumes/Kindle'), 'scan_device_library', { devicePath: '/Volumes/Kindle' }],
    ['listDeviceBooks', () => api.listDeviceBooks('/Volumes/Kindle'), 'list_device_books', { devicePath: '/Volumes/Kindle' }],
    ['cancelTransfer', () => api.cancelTransfer('job-1'), 'cancel_transfer', { jobId: 'job-1' }],
    ['deleteDeviceBook', () => api.deleteDeviceBook('/Volumes/Kindle/documents/a.azw3'), 'delete_device_book',
        { path: '/Volumes/Kindle/documents/a.azw3' }],
    ['getDeviceReadingProgress', () => api.getDeviceReadingProgress('/Volumes/Kindle', 'kindle'), 'get_device_reading_progress',
        { devicePath: '/Volumes/Kindle', deviceType: 'kindle' }],
    ['httpGet', () => api.httpGet('https://x.test/', { Accept: 'application/json' }, 1000), 'http_get',
        { url: 'https://x.test/', headers: { Accept: 'application/json' }, maxBytes: 1000 }],
    ['startOpds', () => api.startOpds(8042, '/lib', true, 'secret'), 'start_opds',
        { port: 8042, libraryRoot: '/lib', shareOnLan: true, password: 'secret' }],
    ['stopOpds', () => api.stopOpds(), 'stop_opds', {}],
    ['opdsStatus', () => api.opdsStatus(), 'opds_status', {}],
    ['invalidateOpdsCache', () => api.invalidateOpdsCache(), 'invalidate_opds_cache', {}],
]

beforeEach(() => mockInvoke.reset())

describe('typed IPC (services/api)', () => {
    it.each(CALLS)('%s calls its command with the documented arguments', async (_name, call, cmd, args) => {
        mockInvoke.on(cmd, () => 'result')
        await expect(call()).resolves.toBe('result')
        expect(mockInvoke.calls).toEqual([{ cmd, args }])
    })

    it('sends send_books its jobs with the snake_case field names Rust reads', async () => {
        mockInvoke.on('send_books', () => undefined)
        const job = {
            id: 'j', book_id: 'b-1', library_path: '/lib', book_path: '/lib/a.epub',
            needs_conversion: true, target_format: 'azw3', device_path: '/Volumes/Kindle', subfolder: null,
        }
        await api.sendBooks([job])
        expect(mockInvoke.callsTo('send_books')[0].args).toEqual({ jobs: [job] })
    })

    it('sends plain copies of reactive arrays', async () => {
        mockInvoke.on('delete_books', () => ({ deleted: [], failures: [], libraryTrash: null }))
        const ids = reactive(['a', 'b'])
        await api.deleteBooks('/lib', ids)
        const sent = mockInvoke.callsTo('delete_books')[0].args.bookIds
        expect(sent).toEqual(['a', 'b'])
        expect(sent).not.toBe(ids)
    })

    it('lets the backend error through untouched, for errorMessage/errorCode to read', async () => {
        const error = { code: 'NOT_FOUND', message: 'This book is no longer in the library.' }
        mockInvoke.on('rename_book', () => backendError(error))
        await expect(api.renameBook('/lib', 'a', 'x.epub')).rejects.toBe(error)
    })

    it('hands Calibre import progress events to the handler, payload only', async () => {
        const seen: unknown[] = []
        const unlisten = await api.onCalibreImportProgress(p => seen.push(p))
        const payload = { done: 1, total: 3, currentTitle: 'Dune' }
        eventListeners.get('calibre-import-progress')!({ payload })
        expect(seen).toEqual([payload])
        expect(typeof unlisten).toBe('function')
    })
})
