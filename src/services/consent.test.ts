/**
 * Analytics consent gating (decision D2: opt-in). The env-dependent modules
 * read their endpoint at import time, so each test stubs the env and
 * re-imports fresh copies.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'

vi.mock('@tauri-apps/api/core', () => import('@/test/tauri'))

const fetchMock = vi.fn(() => Promise.resolve(new Response('{}')))

/**
 * Fresh module instances. The Tauri mock is read back through the mocked
 * module itself: its factory result is cached by vitest across
 * resetModules, so a fresh `import('@/test/tauri')` would be another copy.
 */
async function freshModules() {
    vi.resetModules()
    const { mockInvoke } = await import('@tauri-apps/api/core') as unknown as typeof import('@/test/tauri')
    mockInvoke.reset()
    mockInvoke.on('set_config', () => undefined)
    const analyticsModule = await import('./analytics')
    const api = await import('./librarium-api')
    const installationId = await import('./installationId')
    const { useAnalyticsStore } = await import('@/stores/analytics')
    setActivePinia(createPinia())
    return { ...analyticsModule, ...api, ...installationId, useAnalyticsStore, mockInvoke }
}

/** The JSON bodies of the analytics events sent so far. */
function sentEvents(): { payload: { name: string, data: Record<string, unknown> } }[] {
    return fetchMock.mock.calls
        .map(c => (c as unknown[])[1] as RequestInit | undefined)
        .filter((init): init is RequestInit => !!init?.body)
        .map(init => JSON.parse(init.body as string) as { payload: { name: string, data: Record<string, unknown> } })
}

beforeEach(() => {
    vi.useFakeTimers()
    vi.stubEnv('VITE_API_URL', 'https://api.test')
    vi.stubEnv('VITE_UMAMI_URL', 'https://umami.test')
    vi.stubEnv('VITE_UMAMI_WEBSITE_ID', 'site')
    vi.stubGlobal('fetch', fetchMock)
    fetchMock.mockClear()
})

afterEach(() => {
    vi.useRealTimers()
    vi.unstubAllEnvs()
    vi.unstubAllGlobals()
})

describe('resolveStoredConsent', () => {
    it('reads an explicit choice, migrates an old opt-out, and asks everyone else', async () => {
        const { resolveStoredConsent } = await freshModules()
        expect(resolveStoredConsent({ analyticsConsent: 'granted' })).toBe('granted')
        expect(resolveStoredConsent({ analyticsConsent: 'denied' })).toBe('denied')
        expect(resolveStoredConsent({ analyticsOptOut: true })).toBe('denied')
        // "Not opted out" is not consent.
        expect(resolveStoredConsent({ analyticsOptOut: false })).toBeNull()
        expect(resolveStoredConsent({})).toBeNull()
    })
})

describe('analytics events', () => {
    it('sends nothing before the user has chosen', async () => {
        const { analytics } = await freshModules()
        analytics.track('app_start')
        analytics.trackPageview('/library')
        await vi.runAllTimersAsync()
        expect(fetchMock).not.toHaveBeenCalled()
    })

    it('sends nothing after the user said no, not even an opt-out event', async () => {
        const { useAnalyticsStore, mockInvoke } = await freshModules()
        const store = useAnalyticsStore()
        store.initFromConfig({})
        expect(store.needsConsent).toBe(true)
        await store.deny()
        const { analytics } = await import('./analytics')
        analytics.track('view_open', { view: 'library' })
        await vi.runAllTimersAsync()
        expect(fetchMock).not.toHaveBeenCalled()
        expect(mockInvoke.callsTo('set_config')[0].args).toEqual({ updates: { analyticsConsent: 'denied' } })
    })

    it('sends events once consent is granted', async () => {
        const { useAnalyticsStore, analytics } = await freshModules()
        await useAnalyticsStore().grant()
        analytics.track('view_open', { view: 'library' })
        await vi.runAllTimersAsync()
        expect(fetchMock).toHaveBeenCalled()
        expect(String((fetchMock.mock.calls[0] as unknown[])[0])).toBe('https://umami.test/api/send')
    })

    it('drops queued events when consent is withdrawn before they are flushed', async () => {
        const { useAnalyticsStore, analytics } = await freshModules()
        const store = useAnalyticsStore()
        await store.grant()
        fetchMock.mockClear()
        analytics.track('view_open', { view: 'store' })
        await store.toggle() // → denied, before the 2 s flush timer fires
        await vi.runAllTimersAsync()
        expect(fetchMock).not.toHaveBeenCalled()
    })

    it('never sends a serial number as the device model', async () => {
        const { useAnalyticsStore, analytics } = await freshModules()
        await useAnalyticsStore().grant()
        fetchMock.mockClear()
        analytics.track('device_connected', { brand: 'kobo', model: 'Kobo N418170012345' })
        await vi.runAllTimersAsync()
        const event = sentEvents().find(e => e.payload.name === 'device_connected')
        expect(event?.payload.data.model).toBe('unknown')
    })

    it('does not ask again someone who already opted out', async () => {
        const { useAnalyticsStore } = await freshModules()
        const store = useAnalyticsStore()
        store.initFromConfig({ analyticsOptOut: true })
        expect(store.consent).toBe('denied')
        expect(store.needsConsent).toBe(false)
    })
})

/** The JSON body of the n-th request sent to `url`. */
function bodyOf(url: string, n = 0): Record<string, unknown> {
    const calls = fetchMock.mock.calls
        .map(c => c as unknown as [string, RequestInit])
        .filter(([u]) => u === url)
    return JSON.parse(calls[n][1].body as string) as Record<string, unknown>
}

const ERROR_URL = 'https://api.test/api/app/report/error'
const FEEDBACK_URL = 'https://api.test/api/app/report/feedback'

describe('reportError', () => {
    it('sends nothing without consent', async () => {
        const { reportError } = await freshModules()
        reportError('SCAN_FAILED', 'import.files', new Error('cannot read /Users/ann/Books'))
        expect(fetchMock).not.toHaveBeenCalled()
    })

    it('sends the code, the place and the backend category, never the message', async () => {
        const { reportError, useAnalyticsStore } = await freshModules()
        await useAnalyticsStore().grant()
        fetchMock.mockClear()
        // What a scrubber would miss: a bare file name, a path outside the
        // usual folders. And an e-mail address.
        reportError('IMPORT_FAILED', 'import.files', {
            code: 'IO',
            message: 'Dune (Frank Herbert).epub can’t be added to the library: /srv/nas/ann/Dune.epub (ann@example.com)',
        })
        expect(fetchMock).toHaveBeenCalledTimes(1)
        expect(String((fetchMock.mock.calls[0] as unknown[])[0])).toBe(ERROR_URL)
        const body = bodyOf(ERROR_URL)
        expect(Object.keys(body).sort()).toEqual(['appUid', 'errorCode', 'errorContent', 'os', 'osVersion', 'version'])
        expect(body.errorCode).toBe('IMPORT_FAILED')
        expect(body.errorContent).toBe('import.files (IO)')
        const raw = JSON.stringify(body)
        for (const leak of ['Dune', 'Herbert', '/srv', 'nas', 'ann', 'example.com', 'library']) {
            expect(raw).not.toContain(leak)
        }
    })

    it('sends only the place for errors that are not the backend’s', async () => {
        const { reportError, useAnalyticsStore } = await freshModules()
        await useAnalyticsStore().grant()
        fetchMock.mockClear()
        reportError('TIDY_UP_FAILED', 'tidyUp.book', new Error('HTTP 503 from https://openlibrary.org/search.json?q=Dune'))
        expect(bodyOf(ERROR_URL).errorContent).toBe('tidyUp.book')
        // A code the backend doesn't have (free text in disguise) isn't sent either.
        reportError('TIDY_UP_FAILED', 'tidyUp.book', { code: 'Dune.epub', message: 'x' })
        expect(bodyOf(ERROR_URL, 1).errorContent).toBe('tidyUp.book')
        expect(JSON.stringify(fetchMock.mock.calls)).not.toContain('Dune')
    })

    it('replaces a code or a place that is not a fixed identifier', async () => {
        const { reportError, useAnalyticsStore } = await freshModules()
        await useAnalyticsStore().grant()
        fetchMock.mockClear()
        reportError('could not open /opt/x/Dune.epub', 'Dune.epub' as never)
        const body = bodyOf(ERROR_URL)
        expect(body.errorCode).toBe('UNKNOWN')
        expect(body.errorContent).toBe('unknown')
    })
})

describe('installation ID', () => {
    beforeEach(() => { localStorage.clear() })

    it('is not created at startup, nor by usage events (which carry no identifier)', async () => {
        const { useAnalyticsStore, analytics, peekInstallationId } = await freshModules()
        useAnalyticsStore().initFromConfig({ analyticsConsent: 'granted' })
        analytics.track('view_open', { view: 'library' })
        await vi.runAllTimersAsync()
        expect(sentEvents().length).toBeGreaterThan(0)
        expect(peekInstallationId()).toBeNull()
        for (const event of sentEvents()) {
            expect(JSON.stringify(event)).not.toMatch(/[0-9a-f]{8}-[0-9a-f]{4}-/)
        }
    })

    it('is created by the first error report after consent, then reused', async () => {
        const { reportError, useAnalyticsStore, peekInstallationId } = await freshModules()
        reportError('IMPORT_FAILED', 'import.files')
        expect(peekInstallationId()).toBeNull()      // no consent: no report, no ID
        await useAnalyticsStore().grant()
        fetchMock.mockClear()
        reportError('IMPORT_FAILED', 'import.files')
        reportError('DELETE_FAILED', 'library.trash')
        const id = peekInstallationId()
        expect(id).toMatch(/^[0-9a-f-]{36}$/)
        expect(bodyOf(ERROR_URL, 0).appUid).toBe(id)
        expect(bodyOf(ERROR_URL, 1).appUid).toBe(id)
    })

    it('may be created by feedback without consent (the user sends it)', async () => {
        const { sendFeedback, peekInstallationId } = await freshModules()
        await sendFeedback('Hello', null)
        const id = peekInstallationId()
        expect(id).not.toBeNull()
        expect(bodyOf(FEEDBACK_URL).appUid).toBe(id)
    })

    it('is deleted when consent is withdrawn; the next one is unrelated', async () => {
        const { reportError, useAnalyticsStore, peekInstallationId } = await freshModules()
        const store = useAnalyticsStore()
        store.initFromConfig({ analyticsConsent: 'granted' })
        reportError('IMPORT_FAILED', 'import.files')
        const first = peekInstallationId()
        expect(first).not.toBeNull()

        await store.toggle()                         // switched off in Settings
        expect(peekInstallationId()).toBeNull()

        await store.toggle()                         // and on again
        reportError('IMPORT_FAILED', 'import.files')
        const second = peekInstallationId()
        expect(second).not.toBeNull()
        expect(second).not.toBe(first)
    })

    it('is deleted by "No thanks" in the consent dialog', async () => {
        const { useAnalyticsStore, peekInstallationId, getInstallationId } = await freshModules()
        const store = useAnalyticsStore()
        store.initFromConfig({})
        getInstallationId()
        await store.deny()
        expect(peekInstallationId()).toBeNull()
    })

    it('found at startup with no recorded choice is dropped', async () => {
        localStorage.setItem('appUuid', 'id-without-consent')
        const { useAnalyticsStore, peekInstallationId } = await freshModules()
        useAnalyticsStore().initFromConfig({})       // never asked
        expect(peekInstallationId()).toBeNull()

        localStorage.setItem('appUuid', 'id-without-consent')
        const again = await freshModules()
        again.useAnalyticsStore().initFromConfig({ analyticsOptOut: true })  // old opt-out
        expect(again.peekInstallationId()).toBeNull()
    })

    it('survives a restart once the user has chosen', async () => {
        localStorage.setItem('appUuid', 'kept')
        const { useAnalyticsStore, peekInstallationId } = await freshModules()
        useAnalyticsStore().initFromConfig({ analyticsConsent: 'granted' })
        expect(peekInstallationId()).toBe('kept')
    })

    it('is the only one: the settings file’s old appUid is cleared at startup and never written', async () => {
        const { useAnalyticsStore, mockInvoke } = await freshModules()
        useAnalyticsStore().initFromConfig({ analyticsConsent: 'granted', appUid: 'f00-old' })
        await vi.runAllTimersAsync()
        expect(mockInvoke.callsTo('set_config').map(c => c.args)).toEqual([{ updates: { appUid: null } }])

        const fresh = await freshModules()
        fresh.useAnalyticsStore().initFromConfig({ analyticsConsent: 'granted', appUid: null })
        fresh.useAnalyticsStore().initFromConfig({})
        await vi.runAllTimersAsync()
        expect(fresh.mockInvoke.callsTo('set_config')).toEqual([])
    })
})

describe('rounded values', () => {
    it('sends the session length as a range', async () => {
        const beacons: Blob[] = []
        vi.spyOn(navigator, 'sendBeacon').mockImplementation((_url: string | URL, data?: BodyInit | null) => {
            beacons.push(data as Blob)
            return true
        })
        const { useAnalyticsStore, analytics } = await freshModules()
        await useAnalyticsStore().grant()
        vi.setSystemTime(Date.now() + 37 * 60_000)  // a 37-minute session
        analytics.trackSessionEnd()
        const events = await Promise.all(beacons.map(async b => JSON.parse(await b.text()) as { payload: { name: string, data: unknown } }))
        const close = events.find(e => e.payload.name === 'app_close')
        expect(close?.payload.data).toEqual({ session_duration_bucket: '15-60' })
    })
})

describe('feedback', () => {
    beforeEach(() => { localStorage.clear() })

    it('sends the message with exactly what the dialog lists, the model made safe', async () => {
        const { sendFeedback, cacheAppVersion, feedbackAttachments } = await freshModules()
        cacheAppVersion('0.6.0')
        const device = { brand: 'kobo', model: 'Kobo N418170012345' }
        await sendFeedback('Great app', device)
        const body = bodyOf(FEEDBACK_URL)
        expect(Object.keys(body).sort()).toEqual(['appUid', 'deviceBrand', 'deviceModel', 'message', 'os', 'osVersion', 'version'])
        expect(body).toMatchObject({ message: 'Great app', version: '0.6.0', deviceBrand: 'kobo', deviceModel: 'unknown' })
        const { appUid: _id, message: _message, ...attachments } = body
        expect(attachments).toEqual(feedbackAttachments(device))
    })

    it('sends no e-reader when none is connected', async () => {
        const { sendFeedback } = await freshModules()
        await sendFeedback('Hi', null)
        expect(bodyOf(FEEDBACK_URL)).toMatchObject({ deviceBrand: null, deviceModel: null })
    })
})
