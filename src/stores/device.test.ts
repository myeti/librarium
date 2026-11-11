import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'

vi.mock('@tauri-apps/api/core', () => import('@/test/tauri'))
vi.mock('@/utils/sounds', () => ({ playNewUser: vi.fn(), playIntro: vi.fn(), playTransferComplete: vi.fn() }))

// Captures the event handlers so tests can emit events.
type Handler = (e: { payload: unknown }) => void
const listeners = new Map<string, Handler>()
vi.mock('@tauri-apps/api/event', () => ({
    listen: vi.fn((name: string, handler: Handler) => { listeners.set(name, handler); return Promise.resolve(() => {}) }),
}))

import { backendError, mockInvoke } from '@/test/tauri'
import { chooseDevice, useDeviceStore } from './device'
import { useLibraryStore } from './library'
import { useNotificationsStore } from './notifications'
import type { Book, ConnectedDevice, DeviceBook, MtpDevice, MtpStatus } from '@/types'

const kindle: ConnectedDevice = {
    id: 'k', name: 'Kindle', mountPath: '/Volumes/Kindle', deviceType: 'kindle', connection: 'massStorage',
    preferredFormat: 'azw3', freeSpaceBytes: 1000, totalSpaceBytes: 4000,
}
const kobo: ConnectedDevice = {
    id: 'o', name: 'Kobo Clara HD', mountPath: '/Volumes/KOBOeReader', deviceType: 'kobo', connection: 'massStorage',
    preferredFormat: 'epub',
}
/** A 2024 Kindle, connected over MTP. */
const mtpKindle: ConnectedDevice = {
    id: 'm', name: 'Kindle Paperwhite', mountPath: 'mtp://usb-20-1.2', deviceType: 'kindle', connection: 'mtp',
    preferredFormat: 'azw3', freeSpaceBytes: 2000, totalSpaceBytes: 8000,
}

function book(id: string, title: string, author: string): Book {
    return {
        id, path: `/lib/${id}.azw3`, filename: `${id}.azw3`, format: 'azw3', sizeBytes: 1,
        dateAdded: '2026-01-01', metadata: { title, author },
    }
}

function onDevice(filename: string, title: string, author: string, libraryId?: string): DeviceBook {
    return { path: `/Volumes/Kindle/documents/${filename}`, filename, title, author, libraryId }
}

function emit(event: string, payload: Record<string, unknown>) {
    listeners.get(event)!({ payload })
}

/** The latest toast. */
function lastNotification() {
    const items = useNotificationsStore().items
    return items[items.length - 1]
}

/** Lets pending promise callbacks (a refresh started by an event) run. */
async function settle() {
    for (let i = 0; i < 5; i++) { await Promise.resolve() }
}

/** The store with `devices` connected (the first one selected) and the library at /lib. */
async function setup(deviceBooks: DeviceBook[] = [], devices: ConnectedDevice[] = [kindle]) {
    const store = useDeviceStore()
    await store.setupProgressListener()
    store.devices = devices.map(d => ({ ...d }))
    store.selectedId = devices[0]?.id ?? null
    store.deviceBooks = deviceBooks
    useLibraryStore().libraryPath = '/lib'
    mockInvoke.on('send_books', () => undefined)
    mockInvoke.on('cancel_transfer', () => undefined)
    mockInvoke.on('scan_device_library', () => [])
    mockInvoke.on('list_device_books', () => deviceBooks)
    mockInvoke.on('repair_device_covers', () => 0)
    return store
}

beforeEach(() => {
    setActivePinia(createPinia())
    mockInvoke.reset()
    listeners.clear()
})

describe('device store transfers', () => {
    it('sends books by library id, without title matching', async () => {
        const store = await setup([onDevice('Dune.azw3', 'Dune', 'Herbert, Frank')])
        await store.sendBooks([book('d', 'Dune', 'Frank Herbert')])
        const [job] = mockInvoke.callsTo('send_books')[0].args.jobs as Record<string, unknown>[]
        expect(job).toEqual({
            id: store.transferJobs[0].id, book_id: 'd', library_path: '/lib', book_path: '/lib/d.azw3',
            needs_conversion: false, target_format: 'azw3', device_path: '/Volumes/Kindle', subfolder: null,
        })
        expect(job).not.toHaveProperty('existing_device_path')
        expect(store.transferJobs[0].devicePath).toBe('/Volumes/Kindle')
    })

    it('sends the file without identity when no library is open', async () => {
        const store = await setup()
        useLibraryStore().libraryPath = null
        await store.sendBooks([book('d', 'Dune', 'Frank Herbert')])
        const [job] = mockInvoke.callsTo('send_books')[0].args.jobs as Record<string, unknown>[]
        expect(job).not.toHaveProperty('book_id')
        expect(job.book_path).toBe('/lib/d.azw3')
    })

    it('rescans the device once, when the whole batch has settled', async () => {
        const store = await setup()
        await store.sendBooks([book('a', 'A', 'X'), book('b', 'B', 'X'), book('c', 'C', 'X')])
        const [a, b, c] = store.transferJobs

        emit('transfer-progress', { jobId: a.id, status: 'done', progress: 100 })
        emit('transfer-progress', { jobId: b.id, status: 'error', progress: 0, errorMessage: 'disk full' })
        expect(mockInvoke.callsTo('list_device_books')).toHaveLength(0)

        emit('transfer-progress', { jobId: c.id, status: 'done', progress: 100 })
        await Promise.resolve()
        expect(mockInvoke.callsTo('list_device_books')).toHaveLength(1)
        expect(mockInvoke.callsTo('scan_device_library')).toHaveLength(1)
        expect(b.errorMessage).toBe('disk full')
    })

    it('waits for the backend to confirm a cancellation', async () => {
        const store = await setup()
        await store.sendBooks([book('a', 'A', 'X')])
        const job = store.transferJobs[0]

        await store.cancelTransfer(job.id)
        expect(mockInvoke.callsTo('cancel_transfer')[0].args).toEqual({ jobId: job.id })
        // Not marked as failed locally: Rust decides whether it could still stop it.
        expect(job.status).toBe('queued')
        expect(job.cancelRequested).toBe(true)

        emit('transfer-progress', { jobId: job.id, status: 'cancelled', progress: 0 })
        expect(job.status).toBe('cancelled')
        expect(job.cancelRequested).toBe(false)
        expect(store.activeJobs).toHaveLength(0)
        expect(store.failedJobs).toHaveLength(0)
    })

    it('still reads the older snake_case event payload', async () => {
        const store = await setup()
        await store.sendBooks([book('a', 'A', 'X')])
        const job = store.transferJobs[0]
        emit('transfer-progress', { job_id: job.id, status: 'error', progress: 0, error_message: 'unplugged' })
        expect(job.status).toBe('error')
        expect(job.errorMessage).toBe('unplugged')
    })

    it('marks the jobs failed when send_books itself is rejected', async () => {
        const store = await setup()
        mockInvoke.on('send_books', () => backendError({ code: 'DEVICE', message: 'Kindle is being ejected' }))
        await store.sendBooks([book('a', 'A', 'X')])
        expect(store.transferJobs[0].status).toBe('error')
        expect(store.transferJobs[0].errorMessage).toBe('Kindle is being ejected')
    })

    it('updates the free space from device-space events', async () => {
        const store = await setup([], [kindle, kobo])
        emit('device-space', { mountPath: '/Volumes/Kindle', freeSpaceBytes: 600, totalSpaceBytes: 4000 })
        expect(store.device?.freeSpaceBytes).toBe(600)
        // A device whose space the OS couldn't read keeps what it had.
        emit('device-space', { mountPath: '/Volumes/Kindle' })
        expect(store.device?.freeSpaceBytes).toBe(600)
        expect(store.devices[1].freeSpaceBytes).toBeUndefined()
    })
})

describe('device store contents', () => {
    it('matches by library id first, then by title and author', async () => {
        const store = await setup([
            onDevice('Something.azw3', 'Renamed', 'Nobody', 'sent'),
            onDevice('Dune.azw3', 'Dune', 'Herbert, Frank'),
        ])
        useLibraryStore().books = [book('sent', 'The Sent Book', 'A'), book('d', 'Dune', 'Frank Herbert')]
        expect(store.isOnDevice(book('sent', 'The Sent Book', 'A'))).toBe(true)
        expect(store.isOnDevice(book('d', 'Dune', 'Frank Herbert'))).toBe(true)
        expect(store.isOnDevice(book('p', 'Poems', 'Sylvia Plath'))).toBe(false)
        // The sent copy is that book only: not another one called "Renamed".
        expect(store.isOnDevice(book('other', 'Renamed', 'Nobody'))).toBe(false)

        store.deviceBooks = []
        expect(store.isOnDevice(book('d', 'Dune', 'Frank Herbert'))).toBe(false)
    })

    it('shares one backend listing between the view and a background refresh', async () => {
        const store = await setup()
        let release!: (books: DeviceBook[]) => void
        mockInvoke.on('list_device_books', () => new Promise<DeviceBook[]>(resolve => { release = resolve }))
        const viewListing = store.listDeviceBooks()
        const refresh = store.refreshDeviceContents()
        await settle()
        expect(store.loadingDeviceBooks).toBe(true)
        // Both asked for the same device's books: the device is read once.
        expect(mockInvoke.callsTo('list_device_books')).toHaveLength(1)
        release([onDevice('Dune.azw3', 'Dune', 'Frank Herbert')])
        await Promise.all([viewListing, refresh])
        expect(store.deviceBooks.map(b => b.filename)).toEqual(['Dune.azw3'])
        expect(store.loadingDeviceBooks).toBe(false)
        // Once settled, a new listing reads the device again.
        mockInvoke.on('list_device_books', () => [])
        await store.listDeviceBooks()
        expect(mockInvoke.callsTo('list_device_books')).toHaveLength(2)
    })

    it('repairs the Kindle covers once per device and library, after the first listing', async () => {
        const store = await setup()
        await store.listDeviceBooks()
        await settle()
        expect(mockInvoke.callsTo('repair_device_covers')).toEqual([
            { cmd: 'repair_device_covers', args: { libraryPath: '/lib', devicePath: kindle.mountPath } },
        ])
        await store.listDeviceBooks()
        await store.refreshDeviceContents()
        await settle()
        expect(mockInvoke.callsTo('repair_device_covers')).toHaveLength(1)
    })

    it('stops polling for good when asked', async () => {
        vi.useFakeTimers()
        const store = useDeviceStore()
        mockInvoke.on('detect_devices', () => [])
        store.startPolling()
        store.startPolling() // a second start doesn't schedule twice
        await vi.advanceTimersByTimeAsync(5_000)
        expect(mockInvoke.callsTo('detect_devices')).toHaveLength(1)
        store.stopPolling()
        await vi.advanceTimersByTimeAsync(60_000)
        expect(mockInvoke.callsTo('detect_devices')).toHaveLength(1)
        vi.useRealTimers()
    })
})

describe('device detection', () => {
    it('chooses the same device, else the one swapped in, else the first', () => {
        const swapped = { ...kindle, id: 'k2' }
        expect(chooseDevice(kindle, [kobo, { ...kindle, freeSpaceBytes: 5 }])?.freeSpaceBytes).toBe(5)
        expect(chooseDevice(kindle, [kobo, swapped])?.id).toBe('k2')
        expect(chooseDevice(kindle, [kobo])?.id).toBe('o')
        expect(chooseDevice(null, [kobo, kindle])?.id).toBe('o')
        expect(chooseDevice(kindle, [])).toBeNull()
    })

    it('reads a device when it appears, and only refreshes its space while it stays', async () => {
        const store = await setup([], [])
        mockInvoke.on('detect_devices', () => [kindle])
        await store.pollDevice()
        expect(store.device?.id).toBe('k')
        expect(mockInvoke.callsTo('list_device_books')).toHaveLength(1)

        mockInvoke.on('detect_devices', () => [{ ...kindle, freeSpaceBytes: 10 }])
        await store.pollDevice()
        expect(store.device?.freeSpaceBytes).toBe(10)
        expect(mockInvoke.callsTo('list_device_books')).toHaveLength(1)
    })

    it('notices a swap at the same mount point and reads the new device', async () => {
        const store = await setup([onDevice('Old.azw3', 'Old', 'A')])
        store.readingProgress = { Old: 50 }
        const swapped = { ...kindle, id: 'another-kindle' }
        mockInvoke.on('detect_devices', () => [swapped])
        mockInvoke.on('list_device_books', () => [onDevice('New.azw3', 'New', 'B')])

        await store.pollDevice()
        expect(store.device?.id).toBe('another-kindle')
        expect(store.deviceBooks.map(b => b.title)).toEqual(['New'])
        expect(store.readingProgress).toEqual({})
    })

    it('forgets a device that left, and never shows a previous device’s books', async () => {
        const store = await setup([onDevice('Old.azw3', 'Old', 'A')])
        mockInvoke.on('detect_devices', () => [])
        await store.pollDevice()
        expect(store.device).toBeNull()
        expect(store.deviceBooks).toEqual([])

        // A failed background poll means "nothing connected", silently.
        store.devices = [{ ...kindle }]
        store.selectedId = 'k'
        mockInvoke.on('detect_devices', () => backendError('boom'))
        await store.pollDevice()
        expect(store.devices).toEqual([])
        expect(useNotificationsStore().items).toEqual([])
    })

    it('keeps the selected device while others come and go, and can switch', async () => {
        const store = await setup([], [kindle])
        mockInvoke.on('detect_devices', () => [kobo, kindle])
        await store.pollDevice()
        expect(store.device?.id).toBe('k')
        expect(store.devices.map(d => d.id)).toEqual(['o', 'k'])

        mockInvoke.on('list_device_books', () => [onDevice('Kobo book.epub', 'Kobo book', 'C')])
        await store.selectDevice('o')
        expect(store.device?.id).toBe('o')
        expect(store.deviceBooks.map(b => b.title)).toEqual(['Kobo book'])
        await store.selectDevice('missing')
        expect(store.device?.id).toBe('o')
    })

    it('ignores a refresh that finishes after the device was swapped', async () => {
        const store = await setup([])
        let release: (books: DeviceBook[]) => void = () => {}
        mockInvoke.on('list_device_books', () => new Promise<DeviceBook[]>(resolve => { release = resolve }))
        const pending = store.refreshDeviceContents()
        store.devices = [{ ...kindle, id: 'another-kindle' }]
        store.selectedId = 'another-kindle'
        release([onDevice('Old.azw3', 'Old', 'A')])
        await pending
        expect(store.deviceBooks).toEqual([])
    })
})

describe('eject', () => {
    it('is not possible while a transfer to the device runs', async () => {
        const store = await setup()
        expect(store.canEject).toBe(true)
        await store.sendBooks([book('a', 'A', 'X')])
        expect(store.canEject).toBe(false)
        mockInvoke.on('eject_device', () => undefined)
        await store.ejectDevice()
        expect(mockInvoke.callsTo('eject_device')).toHaveLength(0)

        emit('transfer-progress', { jobId: store.transferJobs[0].id, status: 'done', progress: 100 })
        expect(store.canEject).toBe(true)
    })

    it('a transfer to another device doesn’t block it', async () => {
        const store = await setup([], [kindle, kobo])
        await store.sendBooks([book('a', 'A', 'X')])
        await store.selectDevice('o')
        expect(store.canEject).toBe(true)
    })

    it('removes the device once ejected, even from a poll that was already running', async () => {
        const store = await setup([onDevice('Dune.azw3', 'Dune', 'A')], [kindle, kobo])
        mockInvoke.on('eject_device', () => undefined)
        mockInvoke.on('list_device_books', () => [])

        await store.ejectDevice()
        expect(mockInvoke.callsTo('eject_device')[0].args).toEqual({ mountPath: '/Volumes/Kindle' })
        expect(store.devices.map(d => d.id)).toEqual(['o'])
        expect(store.device?.id).toBe('o') // the other device is selected
        expect(store.ejecting).toBeNull()
        expect(lastNotification()).toMatchObject({ type: 'success', message: 'Kindle can be unplugged.' })

        // A poll that started before the eject still lists it: stays hidden.
        mockInvoke.on('detect_devices', () => [kindle, kobo])
        await store.pollDevice()
        expect(store.devices.map(d => d.id)).toEqual(['o'])
        // Once a poll no longer sees it, plugging it back in shows it again.
        mockInvoke.on('detect_devices', () => [kobo])
        await store.pollDevice()
        mockInvoke.on('detect_devices', () => [kobo, kindle])
        await store.pollDevice()
        expect(store.devices.map(d => d.id)).toEqual(['o', 'k'])
    })

    it('shows why ejecting failed and keeps the device', async () => {
        const store = await setup()
        mockInvoke.on('eject_device', () => backendError({ code: 'DEVICE', message: 'dissented by PID 42' }))
        await store.ejectDevice()
        expect(store.device?.id).toBe('k')
        expect(store.ejecting).toBeNull()
        expect(store.error).toBe('dissented by PID 42')
        expect(lastNotification()).toMatchObject({
            type: 'error', message: 'Could not eject Kindle: dissented by PID 42',
        })
    })

    it('marks the device as ejecting while the command runs', async () => {
        const store = await setup()
        let finish: () => void = () => {}
        mockInvoke.on('eject_device', () => new Promise<void>(resolve => { finish = resolve }))
        const pending = store.ejectDevice()
        await settle()
        expect(store.ejecting).toBe('/Volumes/Kindle')
        expect(store.canEject).toBe(false)
        finish()
        await pending
        expect(store.ejecting).toBeNull()
    })
})

describe('MTP Kindles', () => {
    /** What `get_mtp_status` answers with MTP support on. */
    function mtpStatus(...devices: MtpDevice[]): MtpStatus {
        return { enabled: true, devices }
    }
    /** `mtpKindle`, connected. */
    const ready: MtpDevice = { mountPath: 'mtp://usb-20-1.2', name: 'Kindle Paperwhite', state: 'ready', deviceId: 'm' }
    /** Another 2024 Kindle, which Image Capture holds. */
    const held: MtpDevice = {
        mountPath: 'mtp://usb-20-3', name: 'Kindle', state: 'claimFailed',
        message: 'Another app is using the Kindle. Quit Image Capture.',
    }
    const mtpBook: DeviceBook = {
        path: 'mtp://usb-20-1.2/documents/Dune - Frank Herbert.azw3', filename: 'Dune - Frank Herbert.azw3',
        title: 'Dune', author: 'Frank Herbert', sizeBytes: 1,
    }

    it('asks for their status with each poll, and explains only those that aren’t connected', async () => {
        const store = await setup([], [])
        mockInvoke.on('detect_devices', () => [kindle, mtpKindle])
        mockInvoke.on('get_mtp_status', () => mtpStatus(ready, held))
        await store.pollDevice()
        expect(mockInvoke.callsTo('get_mtp_status')).toHaveLength(1)
        expect(store.devices.map(d => d.id)).toEqual(['k', 'm'])
        expect(store.mtpDevices).toEqual([ready, held])
        expect(store.mtpNotices).toEqual([held])
    })

    it('shows a detected Kindle as a device, whatever a status taken a moment apart says', async () => {
        const store = await setup([], [])
        mockInvoke.on('detect_devices', () => [mtpKindle])
        mockInvoke.on('get_mtp_status', () => mtpStatus({ ...ready, state: 'connecting', deviceId: undefined }))
        await store.pollDevice()
        expect(store.device?.id).toBe('m')
        expect(store.deviceIsMtp).toBe(true)
        expect(store.mtpNotices).toEqual([])

        // Connected but not detected yet: nothing to explain either (the next poll shows it).
        mockInvoke.on('detect_devices', () => [])
        mockInvoke.on('get_mtp_status', () => mtpStatus(ready))
        await store.pollDevice()
        expect(store.mtpNotices).toEqual([])
    })

    it('has nothing to explain when the status can’t be read, silently', async () => {
        const store = await setup([], [])
        mockInvoke.on('detect_devices', () => [kindle])
        mockInvoke.on('get_mtp_status', () => mtpStatus(held))
        await store.pollDevice()
        expect(store.mtpNotices).toHaveLength(1)

        mockInvoke.on('get_mtp_status', () => backendError({ code: 'INTERNAL', message: 'boom' }))
        await store.pollDevice()
        expect(store.mtpNotices).toEqual([])
        expect(store.device?.id).toBe('k') // detection itself is unaffected
        expect(store.deviceIsMtp).toBe(false)
        expect(useNotificationsStore().items).toEqual([])

        // The dev mock backend answers commands it doesn't know with null.
        mockInvoke.on('get_mtp_status', () => mtpStatus(held))
        await store.pollDevice()
        mockInvoke.on('get_mtp_status', () => null)
        await store.pollDevice()
        expect(store.mtpDevices).toEqual([])
    })

    it('polls their status on the detection timer, and with a manual detection', async () => {
        vi.useFakeTimers()
        const store = useDeviceStore()
        mockInvoke.on('detect_devices', () => [])
        mockInvoke.on('get_mtp_status', () => mtpStatus({ ...held, state: 'mtpDisabled' }))
        store.startPolling()
        await vi.advanceTimersByTimeAsync(5_000)
        expect(mockInvoke.callsTo('get_mtp_status')).toHaveLength(1)
        expect(store.mtpNotices.map(n => n.state)).toEqual(['mtpDisabled'])
        await vi.advanceTimersByTimeAsync(5_000)
        expect(mockInvoke.callsTo('get_mtp_status')).toHaveLength(2)
        store.stopPolling()
        vi.useRealTimers()

        mockInvoke.on('get_mtp_status', () => mtpStatus(held))
        await store.detectDevice()
        expect(mockInvoke.callsTo('get_mtp_status')).toHaveLength(3)
        expect(store.mtpNotices).toEqual([held])
    })

    it('says an ejected MTP Kindle is safe to unplug', async () => {
        const store = await setup([], [mtpKindle])
        mockInvoke.on('eject_device', () => undefined)
        await store.ejectDevice()
        expect(mockInvoke.callsTo('eject_device')[0].args).toEqual({ mountPath: 'mtp://usb-20-1.2' })
        expect(store.device).toBeNull()

        // Released: gone from detection, `ejected` in the status until unplugged.
        mockInvoke.on('detect_devices', () => [])
        mockInvoke.on('get_mtp_status', () => mtpStatus({
            mountPath: 'mtp://usb-20-1.2', name: 'Kindle Paperwhite', state: 'ejected', message: 'Ejected: safe to unplug.',
        }))
        await store.pollDevice()
        expect(store.mtpNotices.map(n => n.state)).toEqual(['ejected'])
    })

    it('lists an MTP Kindle once per refresh, and doesn’t read its reading progress', async () => {
        const store = await setup([], [])
        mockInvoke.on('list_device_books', () => [mtpBook])
        mockInvoke.on('detect_devices', () => [mtpKindle])
        mockInvoke.on('get_mtp_status', () => mtpStatus(ready))
        await store.pollDevice()
        expect(mockInvoke.callsTo('list_device_books').map(c => c.args)).toEqual([{ devicePath: 'mtp://usb-20-1.2' }])
        expect(mockInvoke.callsTo('scan_device_library')).toHaveLength(0)
        expect(store.deviceFiles).toEqual(['Dune - Frank Herbert.azw3'])
        expect(store.deviceBooks).toEqual([mtpBook])

        store.readingProgress = { Old: 10 }
        await store.loadReadingProgress()
        expect(mockInvoke.callsTo('get_device_reading_progress')).toHaveLength(0)
        expect(store.readingProgress).toEqual({})
    })

    it('is loading while a listing runs, and rejects when the Kindle can’t be listed', async () => {
        const store = await setup([], [mtpKindle])
        let release: () => void = () => {}
        const gate = new Promise<void>(resolve => { release = resolve })
        mockInvoke.on('list_device_books', async () => {
            await gate
            backendError({ code: 'DEVICE', message: 'The Kindle is busy' })
        })
        const pending = store.listDeviceBooks()
        await settle()
        expect(store.loadingDeviceBooks).toBe(true)
        release()
        await expect(pending).rejects.toEqual({ code: 'DEVICE', message: 'The Kindle is busy' })
        expect(store.loadingDeviceBooks).toBe(false)

        // The background refresh only logs it (the device view shows it when it lists).
        const logged = vi.spyOn(console, 'error').mockImplementation(() => {})
        await store.refreshDeviceContents()
        expect(logged).toHaveBeenCalled()
        expect(store.loadingDeviceBooks).toBe(false)
        expect(useNotificationsStore().items).toEqual([])
    })

    it('drops a listing that ends after another device was selected', async () => {
        const store = await setup([], [mtpKindle, kobo])
        let release: (books: DeviceBook[]) => void = () => {}
        mockInvoke.on('list_device_books', () => new Promise<DeviceBook[]>(resolve => { release = resolve }))
        const pending = store.listDeviceBooks()
        store.selectedId = 'o'
        release([mtpBook])
        await pending
        expect(store.deviceBooks).toEqual([])
    })
})
