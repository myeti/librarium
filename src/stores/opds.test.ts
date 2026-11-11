import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'

vi.mock('@tauri-apps/api/core', () => import('@/test/tauri'))
import { backendError, mockInvoke } from '@/test/tauri'
import { parsePort, useOpdsStore, OPDS_USERNAME, svgDataUrl, toOpds2Url } from './opds'
import type { OpdsStatus } from '@/types'

const status = (lanUrls: string[] = []): OpdsStatus => ({
    running: true, port: 8042, localUrl: 'http://127.0.0.1:8042/opds', lanUrls,
})

beforeEach(() => {
    setActivePinia(createPinia())
    mockInvoke.reset()
    localStorage.clear()
    mockInvoke.on('set_config', () => undefined)
    mockInvoke.on('stop_opds', () => undefined)
})

describe('opds store', () => {
    it('is local-only by default and sends no password', async () => {
        const store = useOpdsStore()
        store.initFromConfig({})
        store.enabled = true
        mockInvoke.on('start_opds', () => status())
        await store.setLibraryRoot('/root/ann')

        expect(mockInvoke.callsTo('start_opds')[0].args).toEqual({
            port: 8042, libraryRoot: '/root/ann', shareOnLan: false, password: null,
        })
        expect(store.running).toBe(true)
        expect(store.localUrl).toBe('http://127.0.0.1:8042/opds')
    })

    it('generates a password when LAN sharing is turned on, and restarts with it', async () => {
        const store = useOpdsStore()
        store.initFromConfig({})
        store.enabled = true
        mockInvoke.on('start_opds', ({ shareOnLan }) => status(shareOnLan ? ['http://192.168.1.10:8042/opds'] : []))
        await store.setLibraryRoot('/root/ann')

        await store.setShareOnLan(true)

        expect(store.password).toMatch(/^[A-Za-z2-9]{20}$/)
        const starts = mockInvoke.callsTo('start_opds')
        const lastStart = starts[starts.length - 1].args
        expect(lastStart).toMatchObject({ shareOnLan: true, password: store.password })
        expect(store.lanUrls).toEqual(['http://192.168.1.10:8042/opds'])
        expect(store.username).toBe(OPDS_USERNAME)
        expect(mockInvoke.callsTo('set_config').map(c => c.args.updates)).toContainEqual({ opdsPassword: store.password })
    })

    it('restarts on the new user’s library and stops when no user is selected', async () => {
        const store = useOpdsStore()
        store.enabled = true
        mockInvoke.on('start_opds', () => status())
        await store.setLibraryRoot('/root/ann')
        await store.setLibraryRoot('/root/bob')
        expect(mockInvoke.callsTo('start_opds').map(c => c.args.libraryRoot)).toEqual(['/root/ann', '/root/bob'])

        await store.setLibraryRoot(null)
        expect(mockInvoke.callsTo('stop_opds')).toHaveLength(1)
        expect(store.running).toBe(false)
    })

    it('does not start while disabled', async () => {
        const store = useOpdsStore()
        store.enabled = false
        await store.setLibraryRoot('/root/ann')
        expect(mockInvoke.callsTo('start_opds')).toHaveLength(0)
    })

    it('keeps the error when the server cannot start', async () => {
        const store = useOpdsStore()
        store.enabled = true
        mockInvoke.on('start_opds', () => backendError('port 8042 is in use'))
        await store.setLibraryRoot('/root/ann')
        expect(store.running).toBe(false)
        expect(store.error).toBe('port 8042 is in use')
    })

    it('reads the real server state from the backend', async () => {
        const store = useOpdsStore()
        store.running = true
        mockInvoke.on('opds_status', () => null)
        await store.refreshStatus()
        expect(store.running).toBe(false)
        expect(store.status).toBeNull()

        mockInvoke.on('opds_status', () => ({ ...status(['http://192.168.1.10:8042/opds']), running: false }))
        await store.refreshStatus()
        expect(store.running).toBe(false)
        expect(store.lanUrls).toEqual(['http://192.168.1.10:8042/opds'])

        mockInvoke.on('opds_status', () => status())
        await store.refreshStatus()
        expect(store.running).toBe(true)

        // A failure keeps what is known.
        mockInvoke.on('opds_status', () => backendError({ code: 'INTERNAL', message: 'boom' }))
        await store.refreshStatus()
        expect(store.running).toBe(true)
    })

    it('gives the OPDS 2 address of the shown catalog URL', async () => {
        expect(toOpds2Url('http://127.0.0.1:8042/opds')).toBe('http://127.0.0.1:8042/opds/v2')
        expect(toOpds2Url('http://127.0.0.1:8042/opds/')).toBe('http://127.0.0.1:8042/opds/v2')
        const store = useOpdsStore()
        store.enabled = true
        mockInvoke.on('start_opds', ({ shareOnLan }) => status(shareOnLan ? ['http://192.168.1.10:8042/opds'] : []))
        await store.setLibraryRoot('/root/ann')
        expect(store.opds2Url).toBe('http://127.0.0.1:8042/opds/v2')
        await store.setShareOnLan(true)
        expect(store.opds2Url).toBe('http://192.168.1.10:8042/opds/v2')
    })
})

describe('opds QR code', () => {
    const LAN_URL = 'http://192.168.1.10:8042/opds'
    const SVG = '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><path d="M0 0h1v1H0z"/></svg>'

    /** A store running in LAN mode, as after the user turned sharing on. */
    async function sharingStore() {
        const store = useOpdsStore()
        store.initFromConfig({ opdsShareOnLan: true, opdsPassword: 'pw-not-in-qr' })
        store.enabled = true
        mockInvoke.on('start_opds', ({ shareOnLan }) => status(shareOnLan ? [LAN_URL, 'http://10.0.0.2:8042/opds'] : []))
        await store.setLibraryRoot('/root/ann')
        return store
    }

    it('encodes the first LAN address, never the password, as an image data URL', async () => {
        mockInvoke.on('opds_qr_svg', () => SVG)
        const store = await sharingStore()
        expect(store.qrUrl).toBe(LAN_URL)

        await store.refreshQr()
        const calls = mockInvoke.callsTo('opds_qr_svg')
        expect(calls.map(c => c.args)).toEqual([{ url: LAN_URL }])
        expect(JSON.stringify(calls)).not.toContain('pw-not-in-qr')
        expect(store.qrDataUrl).toBe(svgDataUrl(SVG))
        expect(store.qrDataUrl?.startsWith('data:image/svg+xml;charset=utf-8,')).toBe(true)
        expect(decodeURIComponent(store.qrDataUrl!.split(',').slice(1).join(','))).toBe(SVG)

        // Same address: not made again.
        await store.refreshQr()
        expect(mockInvoke.callsTo('opds_qr_svg')).toHaveLength(1)
    })

    it('shows nothing in local mode or once stopped', async () => {
        mockInvoke.on('opds_qr_svg', () => SVG)
        const local = useOpdsStore()
        local.enabled = true
        mockInvoke.on('start_opds', () => status())
        await local.setLibraryRoot('/root/ann')
        expect(local.qrUrl).toBeNull()
        await local.refreshQr()
        expect(mockInvoke.callsTo('opds_qr_svg')).toHaveLength(0)
        expect(local.qrDataUrl).toBeNull()

        setActivePinia(createPinia())
        const store = await sharingStore()
        await store.refreshQr()
        expect(store.qrDataUrl).not.toBeNull()
        await store.toggle() // disable: stops the server
        expect(store.qrUrl).toBeNull()
        expect(store.qrDataUrl).toBeNull()
    })

    it('drops a code made for an address that is no longer shown', async () => {
        let finish: (svg: string) => void = () => {}
        mockInvoke.on('opds_qr_svg', () => new Promise<string>(resolve => { finish = resolve }))
        const store = await sharingStore()
        const pending = store.refreshQr()
        await store.stop()
        finish(SVG)
        await pending
        expect(store.qrSvg).toBeNull()
        expect(store.qrDataUrl).toBeNull()
    })

    it('leaves no code when it cannot be made', async () => {
        const errors = vi.spyOn(console, 'error').mockImplementation(() => {})
        mockInvoke.on('opds_qr_svg', () => backendError({ code: 'INVALID_INPUT', message: 'too long' }))
        const store = await sharingStore()
        await store.refreshQr()
        expect(store.qrDataUrl).toBeNull()
        expect(errors).toHaveBeenCalled()
        errors.mockRestore()
    })
})

describe('opds port', () => {
    it('accepts only whole numbers from 1024 to 65535 (never NaN)', () => {
        expect(parsePort('8042')).toBe(8042)
        expect(parsePort(' 1024 ')).toBe(1024)
        expect(parsePort(65535)).toBe(65535)
        for (const bad of ['', 'abc', 'NaN', '80', '65536', '8042.5', '1e4', '-8042', null, undefined]) {
            expect(parsePort(bad), String(bad)).toBeNull()
        }
    })

    it('falls back to the default when the saved port is damaged', () => {
        localStorage.setItem('opdsPort', 'NaN')
        setActivePinia(createPinia())
        expect(useOpdsStore().port).toBe(8042)
        localStorage.removeItem('opdsPort')
    })

    it('ignores an invalid port instead of saving it', async () => {
        const store = useOpdsStore()
        await store.setPort(Number.NaN)
        expect(store.port).toBe(8042)
        expect(localStorage.getItem('opdsPort')).not.toBe('NaN')
    })
})

describe('opds cache', () => {
    it('only invalidates the cache of a running server', async () => {
        const store = useOpdsStore()
        mockInvoke.on('invalidate_opds_cache', () => undefined)
        await store.invalidateCache()
        expect(mockInvoke.callsTo('invalidate_opds_cache')).toHaveLength(0)
        store.running = true
        await store.invalidateCache()
        expect(mockInvoke.callsTo('invalidate_opds_cache')).toHaveLength(1)
    })
})
