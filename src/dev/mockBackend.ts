/**
 * Mock backend for UI development without Tauri (`npm run vue:dev:mock`).
 *
 * The frontend normally talks to the Rust backend through Tauri's IPC. In a
 * plain browser that bridge doesn't exist, so every screen errors out. This
 * module installs Tauri's own IPC mock (`@tauri-apps/api/mocks`) with fixture
 * data — two users, a few dozen books, a connected Kindle — so views can be
 * opened, clicked through and screenshotted.
 *
 * It is only loaded when `VITE_MOCK_BACKEND === '1'` (the `mock` Vite mode,
 * see `.env.mock`); production builds never include it.
 *
 * The data is deliberately plain: covers are absent (cards show their
 * placeholder), writes are applied in memory only and forgotten on reload.
 * Unknown commands log a warning and resolve to `null`. `?mtp=<state>` adds
 * an MTP Kindle (see `MTP_STATE`); `?device=none` unplugs the Kindle;
 * `?transfers=1` starts with a few transfers in the Activity list (see
 * `mockTransfers`). File pickers answer a folder, or a backup for a zip
 * picker; a backup or a restore takes a few seconds (see `runMockBackup`).
 *
 * `?showcase=<user>` shows a real library instead of the fixtures, with
 * its covers (the README's screenshots): see `SHOWCASE`.
 */
import { mockIPC, mockWindows } from '@tauri-apps/api/mocks'
import { emit } from '@tauri-apps/api/event'
import type {
    BackupManifest, BackupProgress, Book, BookFormat, ConnectedDevice, DeviceBook, MtpDeviceState, MtpStatus, TransferJob,
} from '@/types'
import { useDeviceStore } from '@/stores/device'

const ROOT = '/mock/Librarium'

/**
 * `?showcase=<user>`: that user's real library, read from a copy of its
 * index and cover thumbnails in `debug/showcase/<user>/` (`library.json`
 * from `.librarium/`, and `.covers/thumbs/`; `debug/` is git-ignored, and
 * the books themselves aren't needed). The only user, in English, with no
 * e-reader plugged in. Covers are served by Vite from that folder.
 */
const SHOWCASE = new URLSearchParams(location.search).get('showcase')
const SHOWCASE_DIR = SHOWCASE ? `/debug/showcase/${encodeURIComponent(SHOWCASE)}` : ''

const TITLES: [title: string, author: string, series?: string][] = [
    ['Dungeon Crawler Carl', 'Matt Dinniman', 'Dungeon Crawler Carl'],
    ['Carl’s Doomsday Scenario', 'Matt Dinniman', 'Dungeon Crawler Carl'],
    ['The Dungeon Anarchist’s Cookbook', 'Matt Dinniman', 'Dungeon Crawler Carl'],
    ['L’Œil de la veuve du chaos', 'Matt Dinniman', 'Dungeon Crawler Carl'],
    ['Le Petit Prince', 'Antoine de Saint-Exupéry'],
    ['Dune', 'Frank Herbert', 'Dune'],
    ['Le Messie de Dune', 'Frank Herbert', 'Dune'],
    ['Les Enfants de Dune', 'Frank Herbert', 'Dune'],
    ['Fondation', 'Isaac Asimov', 'Fondation'],
    ['Fondation et Empire', 'Isaac Asimov', 'Fondation'],
    ['Seconde Fondation', 'Isaac Asimov', 'Fondation'],
    ['Le Seigneur des anneaux : La Communauté de l’anneau', 'J. R. R. Tolkien', 'Le Seigneur des anneaux'],
    ['Les Deux Tours', 'J. R. R. Tolkien', 'Le Seigneur des anneaux'],
    ['Le Retour du roi', 'J. R. R. Tolkien', 'Le Seigneur des anneaux'],
    ['Madame Bovary', 'Gustave Flaubert'],
    ['Les Misérables', 'Victor Hugo'],
    ['Notre-Dame de Paris', 'Victor Hugo'],
    ['Le Comte de Monte-Cristo', 'Alexandre Dumas'],
    ['Les Trois Mousquetaires', 'Alexandre Dumas'],
    ['Pride and Prejudice', 'Jane Austen'],
    ['A Very Long Title That Keeps Going To Check How Cards Truncate Their Text Gracefully', 'Someone With A Long Name & A Co-Author'],
    ['Neuromancer', 'William Gibson', 'Sprawl'],
    ['Snow Crash', 'Neal Stephenson'],
    ['The Left Hand of Darkness', 'Ursula K. Le Guin', 'Hainish Cycle'],
    ['Astérix le Gaulois', 'René Goscinny & Albert Uderzo', 'Astérix'],
    ['Tintin au Tibet', 'Hergé', 'Les Aventures de Tintin'],
    ['', ''],
    ['Война и мир', 'Лев Толстой'],
    ['三体', '刘慈欣'],
]
const FORMATS: BookFormat[] = ['epub', 'epub', 'epub', 'azw3', 'pdf', 'mobi', 'cbz']

function makeBooks(user: string): Book[] {
    return TITLES.map(([title, author, series], i) => {
        const format = /Astérix|Tintin/.test(title) ? 'cbz' : FORMATS[i % FORMATS.length]
        const filename = `${title || `untitled-${i}`} - ${author || 'unknown'}.${format}`.replace(/[/:]/g, '_')
        const inSeries = series ? TITLES.filter(t => t[2] === series).findIndex(t => t[0] === title) + 1 : undefined
        return {
            id: `${user}-${String(i).padStart(3, '0')}`,
            path: `${ROOT}/${user}/${filename}`,
            filename,
            format,
            sizeBytes: 300_000 + i * 487_311,
            dateAdded: new Date(Date.UTC(2026, 0, 1 + i * 9)).toISOString(),
            metadata: {
                title,
                author,
                series,
                seriesIndex: inSeries,
                language: /[а-я]/i.test(title) ? 'ru' : /[一-鿿]/.test(title) ? 'zh' : /[éèàœ’ô]|Le |Les |La /.test(title) ? 'fr' : 'en',
                description: i % 3 === 0 ? 'A short description so the detail panel has something to show. Lorem ipsum dolor sit amet.' : undefined,
                rating: i % 5 === 0 ? 4 : undefined,
                categories: i % 4 === 0 ? ['Science-fiction'] : i % 4 === 1 ? ['Fantasy', 'Classique'] : [],
            },
        }
    })
}

const state = {
    config: {
        libraryRoot: ROOT,
        libraryRootOverride: ROOT,
        analyticsConsent: 'denied',
        language: SHOWCASE ? 'en' : 'fr',
        ...(SHOWCASE ? { lang: 'en' } : {}),
    } as Record<string, unknown>,
    users: SHOWCASE ? [SHOWCASE] : ['Alice', 'Invité'],
    books: new Map<string, Book[]>(),
    device: {
        id: 'mock-kindle-1',
        name: 'Kindle Paperwhite',
        mountPath: '/Volumes/Kindle',
        deviceType: 'kindle',
        connection: 'massStorage',
        preferredFormat: 'azw3',
        model: 'Paperwhite',
        freeSpaceBytes: 3_100_000_000,
        totalSpaceBytes: 6_400_000_000,
    } as ConnectedDevice,
}

/**
 * `?mtp=<state>` in the address adds a 2024 Kindle in that `get_mtp_status`
 * state (`mtpDisabled`, `connecting`, `claimFailed`, `busy`, `error`,
 * `ejected`; `ready` also connects it, over MTP, and shows it first), to see
 * how the UI explains each one.
 */
const MTP_STATE = new URLSearchParams(location.search).get('mtp') as MtpDeviceState | null
/** `?device=none`: no mass-storage Kindle plugged in (the sidebar's "no e-reader" state). */
const NO_DEVICE = new URLSearchParams(location.search).get('device') === 'none' || SHOWCASE !== null
/** `?transfers=1`: `mockTransfers()` returns sample jobs for the Activity popover. */
export const MOCK_TRANSFERS = new URLSearchParams(location.search).get('transfers') === '1'
/**
 * `?library=denied`: the library root can't be read (macOS privacy settings
 * for network volumes); `?library=unavailable`: it isn't there (NAS not
 * mounted); `?library=io`: reading it fails. To see how the user picker and
 * the library view explain each one.
 */
const LIBRARY_STATE = new URLSearchParams(location.search).get('library')

/** Fail like the backend does for the library state `?library=` asks for. */
function libraryFailure(folder: string): void {
    if (LIBRARY_STATE === 'denied') {
        fail('PERMISSION_DENIED', `Librarium isn't allowed to open the library folder "${folder}". In System Settings › Privacy & Security › Files and Folders, allow Librarium to access Network Volumes (or Removable Volumes, for a USB drive), then try again.`)
    }
    if (LIBRARY_STATE === 'unavailable') {
        fail('LIBRARY_UNAVAILABLE', `The library folder "${folder}" can't be reached. Is the drive or NAS connected?`)
    }
    if (LIBRARY_STATE === 'io') {
        fail('IO', `Could not read the folder ${folder}: Input/output error`)
    }
}
const MTP_MESSAGES: Partial<Record<MtpDeviceState, string>> = {
    mtpDisabled: 'This Kindle connects over MTP, which Librarium supports experimentally. Turn on "MTP Kindles (experimental)" in Settings to use it.',
    claimFailed: 'Another app is using the Kindle. Quit Image Capture, Photos, Android File Transfer or OpenMTP, then unplug and replug the Kindle.',
    busy: 'The Kindle keeps answering "busy". Retrying.',
    error: 'The Kindle stopped answering (USB error). Retrying.',
    ejected: 'Ejected: safe to unplug.',
}
const mtpKindle: ConnectedDevice = {
    id: 'mock-kindle-mtp',
    name: 'Kindle Paperwhite',
    mountPath: 'mtp://usb-20-1.2',
    deviceType: 'kindle',
    connection: 'mtp',
    preferredFormat: 'azw3',
    model: 'Kindle Paperwhite',
    freeSpaceBytes: 9_800_000_000,
    totalSpaceBytes: 14_000_000_000,
}

function connectedDevices(): ConnectedDevice[] {
    const usb = NO_DEVICE ? [] : [{ ...state.device }]
    return MTP_STATE === 'ready' ? [{ ...mtpKindle }, ...usb] : usb
}

function libraryOf(path: string): Book[] {
    const user = path.split('/').pop() ?? 'Alice'
    if (!state.books.has(user)) { state.books.set(user, makeBooks(user)) }
    return state.books.get(user)!
}

/** Books on the mock Kindle that no library book matches (shown as "only on Kindle"). */
const DEVICE_ONLY: readonly Omit<DeviceBook, 'path'>[] = [
    { filename: 'Germinal.mobi', title: 'Germinal', author: 'Émile Zola', sizeBytes: 1_480_000 },
    { filename: 'Anna Karenine.azw3', title: 'Anna Karénine', author: 'Léon Tolstoï', sizeBytes: 2_950_000 },
    { filename: 'Le Horla.pdf', title: '', author: '', sizeBytes: 610_000 },
]

/** Files removed from the mock Kindle this session (by path). */
const removedFromDevice = new Set<string>()

function deviceBooks(mountPath = state.device.mountPath): DeviceBook[] {
    const sent = libraryOf(`${ROOT}/Alice`).slice(0, 6).map((b, i) => ({
        path: `${mountPath}/documents/${b.filename.replace(/\.\w+$/, '.azw3')}`,
        filename: b.filename.replace(/\.\w+$/, '.azw3'),
        title: b.metadata.title,
        author: b.metadata.author,
        libraryId: i < 4 ? b.id : undefined,
        sizeBytes: b.sizeBytes,
    }))
    const others = DEVICE_ONLY.map(f => ({ ...f, path: `${mountPath}/documents/${f.filename}` }))
    return [...sent, ...others].filter(b => !removedFromDevice.has(b.path))
}

/** Reading progress by file stem: the first book sent, two of the others. */
function deviceProgress(): Record<string, number> {
    const [first] = deviceBooks()
    return {
        ...(first ? { [first.filename.replace(/\.\w+$/, '')]: 42 } : {}),
        'Germinal': 67,
        'Le Horla': 100,
    }
}

type Args = Record<string, unknown>

/** Reject like the real backend: a plain `{ code, message }` AppError object. */
function fail(code: string, message: string): never {
    // eslint-disable-next-line @typescript-eslint/only-throw-error -- the backend rejects with plain objects, not Errors
    throw { code, message }
}
const str = (v: unknown) => (typeof v === 'string' ? v : '')

/** `?showcase=`: the user's books, from the copied `library.json` (format 2: paths relative to the library). */
async function loadShowcase(): Promise<void> {
    if (!SHOWCASE || state.books.has(SHOWCASE)) { return }
    const response = await fetch(`${SHOWCASE_DIR}/library.json`)
    if (!response.ok) { fail('LIBRARY_UNAVAILABLE', `No showcase library: copy it into debug/showcase/${SHOWCASE}/ (see SHOWCASE in src/dev/mockBackend.ts).`) }
    const index = await response.json() as { books: Record<string, Book> }
    state.books.set(SHOWCASE, Object.values(index.books).map(b => ({ ...b, path: `${ROOT}/${SHOWCASE}/${b.path}` })))
}

/** `?showcase=`: the book's cover thumbnail, when the copy has one. */
async function showcaseCover(bookId: string): Promise<{ coverPath: string, coverVersion: number } | null> {
    const coverPath = `${SHOWCASE_DIR}/.covers/thumbs/${encodeURIComponent(bookId)}.jpg`
    const response = await fetch(coverPath, { method: 'HEAD' })
    return response.ok && response.headers.get('content-type')?.startsWith('image/') ? { coverPath, coverVersion: 1 } : null
}

/** What a file picker answers: a folder, or a backup for a zip picker. */
function pickedPath(options: Args | undefined): string | null {
    if (options?.directory) { return '/mock/home/Documents' }
    const filters = (options?.filters ?? []) as { extensions: string[] }[]
    return filters.some(f => f.extensions.includes('zip')) ? `/mock/home/Documents/${MOCK_BACKUP.name}` : null
}

const MOCK_BACKUP = {
    name: 'Librarium backup 2026-11-10 14.32.zip',
    manifest: {
        app: 'Librarium',
        format: 1,
        appVersion: '0.6.0',
        createdAt: '2026-11-10T13:32:00Z',
        users: ['Alice', 'Invité'],
        files: 1204,
        bytes: 4_830_000_000,
    } satisfies BackupManifest,
}

/** `cancel_backup` was called during the running mock backup. */
let backupCancelled = false

/**
 * A backup or a restore of MOCK_BACKUP's size, in about four seconds:
 * `backup-progress` events, then (a restore) the swap. Resolves with
 * whether it was cancelled.
 */
async function runMockBackup(restore: boolean): Promise<boolean> {
    backupCancelled = false
    const { files, bytes } = MOCK_BACKUP.manifest
    const progress = (phase: BackupProgress['phase'], share: number): Promise<void> => emit('backup-progress', {
        phase, filesDone: Math.round(files * share), filesTotal: files, bytesDone: Math.round(bytes * share), bytesTotal: bytes,
    } satisfies BackupProgress)
    const pause = (ms: number) => new Promise(resolve => setTimeout(resolve, ms))
    await progress('scanning', 0)
    await pause(500)
    for (let step = 0; step <= 20; step++) {
        if (backupCancelled) { return true }
        await progress('copying', step / 20)
        await pause(150)
    }
    if (restore) {
        await progress('replacing', 1)
        await pause(700)
    }
    return false
}
const ids = (v: unknown) => (Array.isArray(v) ? v.filter((x): x is string => typeof x === 'string') : [])

const handlers: Record<string, (args: Args) => unknown> = {
    // App / plugins
    'plugin:app|version': () => '0.0.0-mock',
    'plugin:app|name': () => 'Librarium (mock)',
    'plugin:app|tauri_version': () => '2',
    'plugin:path|resolve_directory': () => '/mock/home',
    'plugin:path|join': (a) => (Array.isArray(a.paths) ? a.paths.map(String).join('/').replace(/\/+/g, '/') : ''),
    'plugin:path|basename': (a) => str(a.path).split('/').pop() ?? '',
    'plugin:path|dirname': (a) => str(a.path).split('/').slice(0, -1).join('/'),
    'plugin:path|sep': () => '/',
    'plugin:updater|check': () => null,
    'plugin:dialog|open': (a) => pickedPath(a.options as Args | undefined),
    'plugin:dialog|save': () => null,
    'plugin:dialog|ask': () => true,
    'plugin:dialog|confirm': () => true,
    'plugin:dialog|message': () => null,

    // Settings
    get_config: () => ({ ...state.config }),
    set_config: (a) => { Object.assign(state.config, (a.updates as Args | undefined) ?? {}) },

    // Users & library
    list_users: () => {
        libraryFailure('Librarium')
        return [...state.users]
    },
    create_user: (a) => { const n = str(a.name); state.users.push(n); return `${ROOT}/${n}` },
    check_path_accessible: () => true,
    library_lock_status: () => ({ otherInstance: null }),
    scan_library: async (a) => {
        if (SHOWCASE) { await loadShowcase() }
        libraryFailure(str(a.path).split('/').pop() ?? '')
        return libraryOf(str(a.path)).map(b => ({ ...b }))
    },
    ensure_cover: (a) => (SHOWCASE ? showcaseCover(str(a.bookId)) : null),
    update_metadata: (a) => {
        const books = libraryOf(str(a.libraryPath))
        const book = books.find(b => b.id === a.bookId)
        if (!book) { fail('NOT_FOUND', 'Book not found') }
        book.metadata = { ...(a.metadata as Book['metadata']) }
        return { ...book }
    },
    batch_update_metadata: (a) => {
        const books = libraryOf(str(a.libraryPath))
        const patch = (a.patch ?? {}) as Partial<Book['metadata']>
        const updated = books.filter(b => ids(a.bookIds).includes(b.id))
        const volumes = (a.seriesIndexes ?? {}) as Record<string, number>
        for (const b of updated) {
            b.metadata = { ...b.metadata, ...patch, ...(b.id in volumes ? { seriesIndex: volumes[b.id] } : {}) }
        }
        return { books: updated.map(b => ({ ...b })), failures: [] }
    },
    delete_books: (a) => {
        const books = libraryOf(str(a.libraryPath))
        const gone = ids(a.bookIds)
        for (const id of gone) { const i = books.findIndex(b => b.id === id); if (i >= 0) { books.splice(i, 1) } }
        return { deleted: gone, failures: [], libraryTrash: null }
    },
    forget_books: (a) => {
        const books = libraryOf(str(a.libraryPath))
        const gone = ids(a.bookIds)
        for (const id of gone) { const i = books.findIndex(b => b.id === id); if (i >= 0) { books.splice(i, 1) } }
        return { forgotten: gone, failures: [] }
    },
    set_favorite: (a) => {
        const wanted = new Set(ids(a.bookIds))
        const books = libraryOf(str(a.libraryPath)).filter(b => wanted.has(b.id))
        for (const b of books) { b.favorite = a.favorite === true }
        return books.map(b => ({ ...b }))
    },
    rename_book: (a) => {
        const book = libraryOf(str(a.libraryPath)).find(b => b.id === a.bookId)
        if (!book) { fail('NOT_FOUND', 'Book not found') }
        book.filename = str(a.newFilename)
        return { ...book }
    },
    reset_book_data: (a) => ({ ...libraryOf(str(a.libraryPath)).find(b => b.id === a.bookId) }),
    import_books: () => [],
    detect_language: () => 'fr',
    reveal_in_explorer: () => null,
    open_log_folder: () => null,
    set_menu_language: () => null,

    // Devices
    detect_devices: () => connectedDevices(),
    detect_device: () => connectedDevices()[0] ?? null,
    get_mtp_status: (): MtpStatus => ({
        enabled: state.config.mtpEnabled === true,
        devices: MTP_STATE ? [{
            mountPath: mtpKindle.mountPath,
            name: MTP_STATE === 'ready' ? mtpKindle.name : 'Kindle',
            state: MTP_STATE,
            message: MTP_MESSAGES[MTP_STATE],
            deviceId: MTP_STATE === 'ready' ? mtpKindle.id : undefined,
        }] : [],
    }),
    list_device_books: (a) => deviceBooks(str(a.devicePath) || undefined),
    delete_device_book: (a) => { removedFromDevice.add(str(a.path)); return null },
    scan_device_library: (a) => deviceBooks(str(a.devicePath) || undefined).map(b => b.filename),
    get_device_reading_progress: () => deviceProgress(),
    send_books: () => null,
    cancel_transfer: () => null,
    eject_device: () => null,
    repair_device_covers: () => 0,

    // OPDS, network, misc
    opds_status: () => null,
    start_opds: (a) => ({ running: true, port: Number(a.port) || 8042, localUrl: `http://127.0.0.1:${Number(a.port) || 8042}/opds`, lanUrls: a.shareOnLan ? [`http://192.168.1.20:${Number(a.port) || 8042}/opds`] : [] }),
    stop_opds: () => null,
    invalidate_opds_cache: () => null,
    opds_qr_svg: () => '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><rect width="10" height="10" fill="#fff"/><rect x="1" y="1" width="3" height="3"/><rect x="6" y="1" width="3" height="3"/><rect x="1" y="6" width="3" height="3"/></svg>',
    http_get: () => ({ status: 200, contentType: 'application/json', body: '{}', finalUrl: '' }),
    detect_calibre_library: () => null,
    list_calibre_books: () => [],

    // Backup
    export_backup: async (a) => {
        const { users, files, bytes } = MOCK_BACKUP.manifest
        const cancelled = await runMockBackup(false)
        return { path: cancelled ? null : `${str(a.destDir)}/${MOCK_BACKUP.name}`, users, files, bytes, cancelled }
    },
    read_backup: () => ({ ...MOCK_BACKUP.manifest }),
    restore_backup: async () => {
        const cancelled = await runMockBackup(true)
        if (!cancelled) { state.users = [...MOCK_BACKUP.manifest.users]; state.books.clear() }
        return { users: [...state.users], files: MOCK_BACKUP.manifest.files, cancelled }
    },
    cancel_backup: () => { backupCancelled = true; return true },
    reveal_backup: () => null,
}

/**
 * `?transfers=1`: a few transfers in every state, for the Activity popover.
 * Call once the app (and Pinia) is mounted.
 */
export function seedMockTransfers(): void {
    if (!MOCK_TRANSFERS) { return }
    const device = useDeviceStore()
    const books = libraryOf(`${ROOT}/Alice`)
    const job = (i: number, status: TransferJob['status'], progress: number, extra: Partial<TransferJob> = {}): TransferJob => ({
        id: `mock-job-${i}`, direction: 'to-device', book: { ...books[i] }, status, progress,
        needsConversion: books[i].format !== 'azw3', targetFormat: 'azw3', devicePath: '/Volumes/Kindle', ...extra,
    })
    device.transferJobs = [
        job(0, 'done', 100),
        job(1, 'error', 30, { errorMessage: 'The e-reader is full (12 MB free, 48 MB needed).' }),
        job(2, 'transferring', 62),
        job(3, 'converting', 20),
        job(4, 'queued', 0),
        { id: 'mock-job-5', direction: 'from-device', deviceTitle: 'Snow Crash', status: 'transferring', progress: 45, needsConversion: false, targetFormat: 'epub' },
    ]
}

/** Install the mocked IPC. Call before the app mounts. */
export function installMockBackend(): void {
    mockWindows('main')
    mockIPC((cmd, args) => {
        const handler = handlers[cmd]
        if (handler) { return handler((args ?? {}) as Args) }
        console.warn(`[mock backend] unhandled command '${cmd}'`, args)
        return null
    }, { shouldMockEvents: true })
    // Showcase covers are files Vite serves: their path is their address.
    if (SHOWCASE) { (window as unknown as { __TAURI_INTERNALS__: { convertFileSrc: (p: string) => string } }).__TAURI_INTERNALS__.convertFileSrc = p => p }
    console.info('[mock backend] installed — data is fake and changes are kept in memory only')
}
