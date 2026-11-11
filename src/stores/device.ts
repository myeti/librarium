import { defineStore } from 'pinia'
import { listen } from '@tauri-apps/api/event'
import type {
    ConnectedDevice, TransferJob, Book, BookFormat, DeviceBook, DeviceSpaceEvent, MtpDevice, MtpStatus,
    TransferProgressEvent, TransferRequest,
} from '@/types'
import * as api from '@/services/api'
import { playTransferComplete } from '@/utils/sounds'
import { useSettingsStore } from '@/stores/settings'
import { useLibraryStore } from '@/stores/library'
import { analytics } from '@/services/analytics'
import { reportError } from '@/services/librarium-api'
import { DeviceIndex } from '@/services/deviceMatch'
import { useNotificationsStore } from '@/stores/notifications'
import { errorMessage } from '@/utils/errors'
import { isMtpDevice } from '@/utils/mtp'
import { t } from '@/i18n'

/** A job in one of these states will not change any more. */
export function isSettled(status: TransferJob['status']): boolean {
    return status === 'done' || status === 'error' || status === 'cancelled'
}

/** Detection polling interval while nothing is connected, and while a device is. */
const POLL_IDLE_MS = 5_000
const POLL_CONNECTED_MS = 10_000

/**
 * The device to show after a detection: the same one if it is still there
 * (same id); else the device now mounted where it was (it was swapped for
 * another one); else the first one detected; else none.
 */
export function chooseDevice(current: ConnectedDevice | null, detected: readonly ConnectedDevice[]): ConnectedDevice | null {
    if (current) {
        const same = detected.find(d => d.id === current.id)
        if (same) { return same }
        const swapped = detected.find(d => d.mountPath === current.mountPath)
        if (swapped) { return swapped }
    }
    return detected[0] ?? null
}

/**
 * The connected e-readers and the transfers to and from them.
 *
 * - **Detection.** `startPolling()` asks the backend every few seconds which
 *   e-readers are plugged in (silently: a failed poll just means "none").
 *   `detectDevice()` is the manual check (sidebar button, Cmd+K), whose
 *   failures are shown. Each poll also brings each device's free space.
 * - **Selection.** Several devices can be connected; `device` is the one
 *   the views show and books are sent to (`selectDevice`). Devices have
 *   stable ids, so a different id at the same mount point is a swap: the
 *   new device's contents are read (`applyDetection`).
 * - **Contents.** When a device is selected, and once a batch of transfers is
 *   over, its file list and book list are read again (`refreshDeviceContents`).
 *   `deviceIndex` indexes them so `isOnDevice(book)` is a few set lookups:
 *   by library id for the books Librarium sent, else title/author/file name.
 * - **Transfers.** `sendBooks()` creates one job per book and hands them to
 *   the backend, by library id, which reports progress through
 *   `transfer-progress` events and the device's new free space through
 *   `device-space` (`setupProgressListener`). A job only changes state when
 *   the backend says so; cancelling asks, and the backend confirms.
 * - **Eject.** `ejectDevice()` safely ejects the selected device, once no
 *   transfer to or from it is running (`canEject`).
 * - **MTP Kindles** (2024+, experimental). Each poll also asks for their
 *   status (`get_mtp_status`). A connected one is a device like the others
 *   (`connection: 'mtp'`); the others (MTP support off, held by another
 *   app, connecting, ejected…) are in `mtpNotices`, for the sidebar to say
 *   why they can't be used. Listing one is slow (a request per book), so
 *   it is listed once per refresh, and its reading progress isn't read.
 */
interface DeviceState {
    /** Every connected e-reader, as of the last detection. */
    devices: ConnectedDevice[]
    /** Id of the device the views show and books are sent to (see the `device` getter). */
    selectedId: string | null
    /**
     * Devices just ejected: hidden until a detection no longer sees them
     * (a poll that started before the eject would bring them back).
     */
    ejectedIds: string[]
    /** Mount path of the device being ejected. */
    ejecting: string | null
    /** A manual detection is running. */
    detecting: boolean
    transferJobs: TransferJob[]
    /** File names of the books on the selected device. */
    deviceFiles: string[]
    deviceBooks: DeviceBook[]
    /** Listings of the selected device's books in progress (see `loadingDeviceBooks`). */
    pendingListings: number
    /** Devices (id + library) whose Kindle covers were repaired this session (`repairCovers`). */
    coversRepaired: string[]
    /** Reading progress (0–100) by file name without its last extension. */
    readingProgress: Record<string, number>
    /** The last failure of a device action, for the views. */
    error: string | null
    /** The pending detection poll (bookkeeping for start/stopPolling). */
    pollHandle: ReturnType<typeof setTimeout> | null
    /**
     * The MTP Kindles plugged in, whatever their state, as of the last
     * poll (`get_mtp_status`). The ready ones are also in `devices`.
     */
    mtpDevices: MtpDevice[]
}

/**
 * Listings in flight, by device mount path. Opening the Device view and the
 * background refresh (a device appearing, a transfer batch ending) often
 * ask for the same device's books at the same moment; sharing one backend
 * listing avoids reading every book on the device twice over USB.
 */
const listingsInFlight = new Map<string, Promise<DeviceBook[]>>()


/** The books on the device at `mountPath`, sharing a listing already running. */
function sharedListing(mountPath: string): Promise<DeviceBook[]> {
    const running = listingsInFlight.get(mountPath)
    if (running) { return running }
    const listing = api.listDeviceBooks(mountPath)
        .finally(() => { listingsInFlight.delete(mountPath) })
    listingsInFlight.set(mountPath, listing)
    return listing
}

export const useDeviceStore = defineStore('device', {
    state: (): DeviceState => ({
        devices: [],
        selectedId: null,
        ejectedIds: [],
        ejecting: null,
        detecting: false,
        transferJobs: [],
        deviceFiles: [],
        deviceBooks: [],
        pendingListings: 0,
        coversRepaired: [],
        readingProgress: {},
        error: null,
        pollHandle: null,
        mtpDevices: [],
    }),

    getters: {
        /** The selected device (see `selectDevice`), or null when none is connected. */
        device: (state): ConnectedDevice | null => state.devices.find(d => d.id === state.selectedId) ?? null,
        isConnected(): boolean { return this.device !== null },
        /** The selected device is an MTP Kindle (experimental; its paths are `mtp://` locators). */
        deviceIsMtp(): boolean { return this.device !== null && isMtpDevice(this.device) },
        /** The selected device's books are being read (the list may be empty meanwhile). */
        loadingDeviceBooks: (state): boolean => state.pendingListings > 0,
        activeJobs: (state): TransferJob[] => state.transferJobs.filter(j => !isSettled(j.status)),
        /** Jobs that are over: done, failed or cancelled (the activity list's "Clear finished"). */
        finishedJobs: (state): TransferJob[] => state.transferJobs.filter(j => isSettled(j.status)),
        failedJobs: (state): TransferJob[] => state.transferJobs.filter(j => j.status === 'error'),

        /**
         * The device's books, indexed for `isOnDevice`; rebuilt when they (or
         * the library's books) change. The library's ids tell a copy sent
         * from this library from one sent from another user's.
         */
        deviceIndex: (state): DeviceIndex =>
            new DeviceIndex(state.deviceBooks, new Set(useLibraryStore().books.map(b => b.id))),

        /**
         * "Already on device": a device book carries the book's library id;
         * else (books Librarium didn't send) the same title AND author, or
         * the same file name (see deviceMatch.ts).
         */
        isOnDevice(): (book: Book) => boolean {
            const index = this.deviceIndex
            return (book: Book) => index.has(book)
        },

        /**
         * Whether the selected device can be ejected now: no eject running,
         * and no transfer to or from it that hasn't settled.
         */
        canEject(): boolean {
            const device = this.device
            if (!device || this.ejecting !== null) { return false }
            return !this.transferJobs.some(j => !isSettled(j.status) && j.devicePath === device.mountPath)
        },

        /**
         * The MTP Kindles plugged in but not usable as devices, for the
         * sidebar to say why: MTP support off, connecting, held by another
         * app, busy, failing, or ejected (safe to unplug). One that is in
         * `devices` is shown as a device whatever its status says (the two
         * calls of a poll can straddle a change of state).
         */
        mtpNotices: (state): MtpDevice[] => state.mtpDevices.filter(m =>
            m.state !== 'ready' && !state.devices.some(d => d.mountPath === m.mountPath)),
    },

    actions: {
        /** Forgets what was read from a device (unplugged, swapped, or detection failed). */
        clearDeviceContents() {
            this.deviceFiles = []
            this.deviceBooks = []
            this.readingProgress = {}
        },

        /**
         * Takes a detection result: keeps the selected device if it is still
         * there (with its new free space), follows a swap or picks another
         * device otherwise, and reads the contents of a newly selected one.
         * Devices just ejected are left out until they are really gone.
         */
        async applyDetection(detected: ConnectedDevice[]) {
            this.ejectedIds = this.ejectedIds.filter(id => detected.some(d => d.id === id))
            const visible = detected.filter(d => !this.ejectedIds.includes(d.id))
            const before = this.device
            const next = chooseDevice(before, visible)
            this.devices = visible
            this.selectedId = next?.id ?? null
            if (!next) {
                if (before) { this.clearDeviceContents() }
                return
            }
            if (before?.id === next.id && before.mountPath === next.mountPath) { return }
            // Another device (or the same one remounted elsewhere): what was
            // read from the previous one doesn't apply.
            this.clearDeviceContents()
            if (before?.id !== next.id) {
                analytics.track('device_connected', { brand: next.deviceType, model: next.model ?? 'unknown' })
            }
            await this.refreshDeviceContents()
        },

        /**
         * Takes a `get_mtp_status` result; `null` (the call failed, or an
         * older backend without MTP) means no MTP Kindle to explain.
         */
        applyMtpStatus(status: MtpStatus | null) {
            this.mtpDevices = Array.isArray(status?.devices) ? status.devices : []
        },

        /** Shows (and sends books to) another connected device. */
        async selectDevice(id: string) {
            if (id === this.selectedId || !this.devices.some(d => d.id === id)) { return }
            this.selectedId = id
            this.clearDeviceContents()
            await this.refreshDeviceContents()
        },

        /** Applies a `device-space` event: the free space after a transfer job. */
        onDeviceSpace(p: DeviceSpaceEvent) {
            for (const d of this.devices) {
                if (d.mountPath !== p.mountPath) { continue }
                if (p.freeSpaceBytes !== undefined) { d.freeSpaceBytes = p.freeSpaceBytes }
                if (p.totalSpaceBytes !== undefined) { d.totalSpaceBytes = p.totalSpaceBytes }
            }
        },

        /**
         * Re-reads the device's file list and book list. Both walk the whole
         * device, so this runs once when a transfer batch has finished, not
         * after every job. An MTP Kindle is listed once (both commands would
         * list the same files over MTP, a request per book): the file names
         * are taken from its books. A failure is only logged here (an MTP
         * listing can reject); the device view shows it when it lists.
         */
        async refreshDeviceContents() {
            if (!this.device) { return }
            const { id, mountPath } = this.device
            const mtp = isMtpDevice(this.device)
            this.pendingListings++
            try {
                let files: string[]
                let books: DeviceBook[]
                if (mtp) {
                    books = await sharedListing(mountPath)
                    files = books.map(b => b.filename)
                } else {
                    [files, books] = await Promise.all([
                        api.scanDeviceLibrary(mountPath),
                        sharedListing(mountPath),
                    ])
                }
                // The device may have been unplugged, swapped or deselected meanwhile.
                if (this.device?.id !== id) { return }
                this.deviceFiles = files
                this.deviceBooks = books
                void this.repairCovers()
            } catch (e) {
                console.error('[device] rescan failed', e)
            } finally {
                this.pendingListings--
            }
        },

        onJobSettled(job: TransferJob) {
            if (job.status === 'done' && job.direction === 'to-device' && job.book) {
                analytics.track('transfer_to_device', {
                    format:         job.targetFormat,
                    converted_from: job.needsConversion ? job.book.format : '',
                    brand:          this.device?.deviceType ?? 'unknown',
                    model:          this.device?.model ?? 'unknown',
                })
            } else if (job.status === 'error') {
                analytics.track('error', { component: 'transfer', error_type: 'transfer_failed' })
            }

            const allSettled = this.transferJobs.every(j => isSettled(j.status))
            if (allSettled) { playTransferComplete() }

            // Rescan once the whole batch is over: each rescan walks the device
            // twice, and doing it per job made large batches crawl.
            const batch = this.transferJobs.filter(j => j.batchId !== undefined && j.batchId === job.batchId)
            if (batch.every(j => isSettled(j.status))) {
                void this.refreshDeviceContents()
            }
        },

        /** Applies one `transfer-progress` event to its job. */
        onProgress(p: TransferProgressEvent) {
            const jobId = p.jobId ?? p.job_id
            const job = this.transferJobs.find(j => j.id === jobId)
            if (!job) { return }
            const wasActive = !isSettled(job.status)
            job.status = p.status
            job.progress = p.progress
            const message = p.errorMessage ?? p.error_message
            if (message) {
                job.errorMessage = message
            }
            if (isSettled(job.status)) {
                job.cancelRequested = false
            }
            if (wasActive && isSettled(job.status)) {
                this.onJobSettled(job)
            }
        },

        /** Listens to the backend's transfer events: `transfer-progress` and `device-space`. */
        async setupProgressListener() {
            await listen<TransferProgressEvent>('transfer-progress', event => this.onProgress(event.payload))
            await listen<DeviceSpaceEvent>('device-space', event => this.onDeviceSpace(event.payload))
        },

        /**
         * Manual detection (sidebar refresh, Cmd+K): failures are shown, unlike
         * background polling. The MTP status is refreshed too (silently).
         */
        async detectDevice() {
            this.detecting = true
            this.error = null
            try {
                const [detected, mtp] = await Promise.all([
                    api.detectDevices(),
                    api.getMtpStatus().catch(() => null),
                ])
                this.applyMtpStatus(mtp)
                await this.applyDetection(detected)
            } catch (e) {
                this.error = errorMessage(e)
                await this.applyDetection([])
                useNotificationsStore().push('error', t('errors.deviceDetectFailed', { error: this.error }))
            } finally {
                this.detecting = false
            }
        },

        async sendBooks(books: Book[]) {
            const device = this.device
            if (!device) { return }
            this.error = null

            const preferredFormat = device.preferredFormat as BookFormat
            // Kobo reads EPUB/PDF/CBZ natively over USB.
            // Kindle only reads AZW3/MOBI/PDF over USB — EPUB must be converted to AZW3.
            const isKobo = device.deviceType === 'kobo'
            const nativeFormats: BookFormat[] = isKobo
                ? ['epub', 'pdf', 'cbz']
                : ['mobi', 'azw3', 'pdf']

            const batchId = crypto.randomUUID()
            const jobs: TransferJob[] = books.map(book => ({
                id: crypto.randomUUID(),
                direction: 'to-device' as const,
                book,
                status: 'queued',
                progress: 0,
                needsConversion: !nativeFormats.includes(book.format),
                targetFormat: nativeFormats.includes(book.format) ? book.format : preferredFormat,
                batchId,
                devicePath: device.mountPath,
            }))

            this.transferJobs.push(...jobs)

            const subfolder = useSettingsStore().deviceSubfolder.trim() || null

            // Books are sent by library id: the backend reads the file from
            // the library index, tags the copy, and finds the copy it sent
            // before through the device's manifest (same folder, so a
            // Kindle's .sdr reading state stays valid; the old file replaced).
            // Without an open library, the file is sent as is, without identity.
            const libraryPath = useLibraryStore().libraryPath
            const requests: TransferRequest[] = books.map((book, i) => ({
                id: jobs[i].id,
                ...(libraryPath ? { book_id: book.id, library_path: libraryPath } : {}),
                book_path: book.path,
                needs_conversion: jobs[i].needsConversion,
                target_format: jobs[i].targetFormat,
                device_path: device.mountPath,
                subfolder,
            }))
            console.log('[sendBooks] invoking send_books with', requests.length, 'job(s)')

            try {
                await api.sendBooks(requests)
            } catch (e) {
                this.error = errorMessage(e)
                console.error('[sendBooks] invoke failed:', e)
                // The backend never started these jobs, so no event will settle them.
                // (The jobs are read back from state: `jobs` holds the plain objects.)
                const ids = new Set(jobs.map(j => j.id))
                for (const job of this.transferJobs) {
                    if (ids.has(job.id) && !isSettled(job.status)) {
                        job.status = 'error'
                        job.errorMessage = this.error
                    }
                }
                useNotificationsStore().push('error', t('errors.sendFailed', { error: this.error }))
            }
        },

        /**
         * Asks the backend to cancel a job. The job is *not* marked here: Rust
         * either skips it (queued) or stops it before the copy, then emits a
         * `cancelled` status. If it was already being copied it finishes, and
         * the event says `done` instead, which is the truth.
         */
        async cancelTransfer(jobId: string) {
            const job = this.transferJobs.find(j => j.id === jobId)
            if (job) { job.cancelRequested = true }
            try {
                await api.cancelTransfer(jobId)
            } catch (e) {
                if (job) { job.cancelRequested = false }
                this.error = errorMessage(e)
                useNotificationsStore().push('error', t('errors.cancelFailed', { error: this.error }))
            }
        },

        /**
         * Forgets the jobs that are over (done, failed or cancelled). Jobs
         * still queued or running stay: the store needs them to follow
         * their progress events.
         */
        clearFinished() {
            this.transferJobs = this.transferJobs.filter(j => !isSettled(j.status))
        },

        /**
         * Re-reads the device's books for the device view; rejects (after
         * reporting) on failure, e.g. `DEVICE` for an MTP Kindle that can't
         * be listed. A listing that ends after the device was swapped or
         * deselected is dropped (an MTP listing can take a while).
         */
        /**
         * Gives the books Librarium sent to this Kindle their home-screen
         * cover if they have none (sent before covers were written). Once per
         * device and library per session; best effort and silent.
         */
        async repairCovers() {
            const device = this.device
            const libraryPath = useLibraryStore().libraryPath
            if (!device || device.deviceType !== 'kindle' || isMtpDevice(device) || !libraryPath) { return }
            const key = `${device.id}\n${libraryPath}`
            if (this.coversRepaired.includes(key)) { return }
            this.coversRepaired.push(key)
            try {
                await api.repairDeviceCovers(libraryPath, device.mountPath)
            } catch (e) {
                console.error('[device] cover repair failed', e)
            }
        },

        async listDeviceBooks() {
            if (!this.device) { return }
            const { id, mountPath } = this.device
            this.pendingListings++
            try {
                const books = await sharedListing(mountPath)
                if (this.device?.id === id) {
                    this.deviceBooks = books
                    void this.repairCovers()
                }
            } catch (e) {
                reportError('LIST_DEVICE_BOOKS_FAILED', 'device.list', e)
                throw e
            } finally {
                this.pendingListings--
            }
        },

        /**
         * Reading progress, best effort: nothing is shown when it can't be
         * read. Not read on an MTP Kindle, where it isn't shown: whether
         * 2024 Kindles expose it over MTP is unverified, and copying it
         * would keep the device's single connection busy.
         */
        async loadReadingProgress() {
            if (!this.device) { return }
            if (isMtpDevice(this.device)) {
                this.readingProgress = {}
                return
            }
            try {
                this.readingProgress = await api.getDeviceReadingProgress(this.device.mountPath, this.device.deviceType)
            } catch {
                this.readingProgress = {}
            }
        },

        async deleteDeviceBook(path: string) {
            try {
                await api.deleteDeviceBook(path)
            } catch (e) {
                reportError('DELETE_DEVICE_BOOK_FAILED', 'device.delete', e)
                throw e
            }
            this.deviceBooks = this.deviceBooks.filter(b => b.path !== path)
            this.deviceFiles = this.deviceFiles.filter(f => !path.endsWith(f))
        },

        /**
         * Copies a device book into the library. Resolves with the imported book
         * (with the Kindle title/author fix applied when possible), or null when
         * the import failed; the failure is shown on the job and as a toast.
         */
        async importFromDevice(deviceBook: DeviceBook, libraryPath: string): Promise<Book | null> {
            this.transferJobs.push({
                id: crypto.randomUUID(),
                direction: 'from-device',
                deviceTitle: deviceBook.title || deviceBook.filename,
                status: 'queued',
                progress: 0,
                needsConversion: false,
                targetFormat: 'epub',
                devicePath: this.device?.mountPath,
            })
            // The reactive copy: changes to it show in the transfer list.
            const job = this.transferJobs[this.transferJobs.length - 1]
            try {
                job.status = 'transferring'
                job.progress = 50
                const imported = await api.importBooks([deviceBook.path], libraryPath)
                job.status = 'done'
                let book: Book | null = imported[0] ?? null
                if (book) {
                    // Kindle personal documents often carry "Title  Author" (two
                    // spaces) in the title field; split it back into two fields.
                    const title = book.metadata.title ?? ''
                    const parts = title.split(/\s{2,}/).map(p => p.trim()).filter(Boolean)
                    if (parts.length >= 2) {
                        const metadata = {
                            ...book.metadata,
                            title: parts[0],
                            author: parts[parts.length - 1],
                            categories: book.metadata.categories ?? [],
                        }
                        try {
                            book = await api.updateMetadata(libraryPath, book.id, metadata)
                        } catch (patchErr) {
                            // The book is imported; only the title cleanup failed.
                            console.error('[import] kindle metadata patch failed', patchErr)
                        }
                    }
                }
                job.progress = 100
                const ext = deviceBook.path.split('.').pop() ?? 'unknown'
                analytics.track('transfer_from_device', {
                    format: ext,
                    brand:  this.device?.deviceType ?? 'unknown',
                    model:  this.device?.model ?? 'unknown',
                })
                return book
            } catch (e) {
                job.status = 'error'
                job.errorMessage = errorMessage(e)
                analytics.track('error', { component: 'transfer_from_device', error_type: 'import_failed' })
                useNotificationsStore().push('error', t('errors.deviceImportFailed', {
                    title: job.deviceTitle ?? '', error: job.errorMessage,
                }))
                return null
            }
        },

        /**
         * One background detection: a device that appeared (or replaced the
         * selected one) is read, one that left is forgotten, and the free
         * space shown is refreshed. The MTP status is asked for at the same
         * time. Either call failing (silently) means "none".
         */
        async pollDevice() {
            const [detected, mtp] = await Promise.all([
                api.detectDevices().catch((): ConnectedDevice[] => []),
                api.getMtpStatus().catch(() => null),
            ])
            this.applyMtpStatus(mtp)
            await this.applyDetection(detected)
        },

        /**
         * Safely ejects the selected device (refused by `canEject` while a
         * transfer to or from it runs). On success it disappears from the
         * list and another connected device, if any, is selected; a failure
         * (`CONFLICT`: a transfer started meanwhile; `DEVICE`: the system
         * couldn't, or can't on Windows) is shown with the backend's reason.
         */
        async ejectDevice() {
            const device = this.device
            if (!device || !this.canEject) { return }
            this.ejecting = device.mountPath
            this.error = null
            try {
                await api.ejectDevice(device.mountPath)
                this.ejectedIds.push(device.id)
                useNotificationsStore().push('success', t('devices.ejected', { name: device.name }))
                await this.applyDetection(this.devices)
            } catch (e) {
                this.error = errorMessage(e)
                useNotificationsStore().push('error', t('devices.ejectFailed', { name: device.name, error: this.error }))
            } finally {
                this.ejecting = null
            }
        },

        schedulePoll() {
            const delay = this.device ? POLL_CONNECTED_MS : POLL_IDLE_MS
            this.pollHandle = setTimeout(() => {
                void this.pollDevice().finally(() => {
                    // Stopped meanwhile: don't start again.
                    if (this.pollHandle !== null) { this.schedulePoll() }
                })
            }, delay)
        },

        startPolling() {
            if (this.pollHandle !== null) { return }
            this.schedulePoll()
        },

        stopPolling() {
            if (this.pollHandle !== null) {
                clearTimeout(this.pollHandle)
                this.pollHandle = null
            }
        },
    },
})
