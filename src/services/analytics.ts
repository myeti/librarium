import { bucketMinutes, safeDeviceModel } from './privacy'

const WEBSITE_ID = import.meta.env.VITE_UMAMI_WEBSITE_ID ?? ''
const UMAMI_URL  = import.meta.env.VITE_UMAMI_URL ?? ''
const ENDPOINT   = `${UMAMI_URL}/api/send`

/**
 * Analytics is opt-in (decision D2). Nothing is sent, or even queued, until
 * the user has explicitly said yes in the consent dialog:
 * - `null`: no choice yet (first run, or an install from before the consent
 *   dialog existed). Events are dropped and the dialog is shown.
 * - `'denied'`: events are dropped. Opting out sends nothing, not even an
 *   "opted out" event.
 * - `'granted'`: events are sent.
 *
 * The choice is persisted in the app config (`analyticsConsent`) by the
 * analytics store; this module only holds the in-memory flag so `track()`
 * can stay synchronous. Error reports (librarium-api.ts `reportError`)
 * follow the same consent.
 */
export type AnalyticsConsent = 'granted' | 'denied'

let _consent: AnalyticsConsent | null = null

/**
 * The consent stored in config, taking the old opt-out flag into account.
 *
 * Before the consent dialog, analytics was on by default with an opt-out
 * switch (`analyticsOptOut`). Someone who switched it off made an explicit
 * choice, which is kept. Everyone else never really chose (the default was
 * on), so they get `null` and are asked once.
 */
export function resolveStoredConsent(cfg: { analyticsConsent?: unknown, analyticsOptOut?: unknown }): AnalyticsConsent | null {
    if (cfg.analyticsConsent === 'granted' || cfg.analyticsConsent === 'denied') { return cfg.analyticsConsent }
    if (cfg.analyticsOptOut === true) { return 'denied' }
    return null
}

/**
 * The values an event may carry: fixed strings (names, formats, ranges) and
 * booleans. No numbers: counts and durations are sent as ranges
 * (`bucketCount`, `bucketMinutes`… in ./privacy.ts and below), never exactly.
 */
export type EventData = Record<string, string | boolean>

interface UmamiEvent {
    name: string
    data: EventData
}

/**
 * Events carry no identifier: no installation ID, no client ID. (Umami
 * groups the events of one visit itself, from the request; see PRIVACY.md.)
 */
class Analytics {
    private readonly sessionStart: number
    private queue:                 UmamiEvent[] = []
    private flushTimer:            ReturnType<typeof setTimeout> | null = null
    private pendingUserProps:      Record<string, string> | null = null

    constructor() {
        this.sessionStart = Date.now()
    }

    /** The current consent; `null` while the user hasn't chosen. */
    get consent(): AnalyticsConsent | null {
        return _consent
    }

    /** True only once the user has explicitly allowed analytics. */
    get hasConsent(): boolean {
        return _consent === 'granted'
    }

    /** Kept for existing callers: "opted out" means "not allowed to send". */
    get optOut(): boolean {
        return !this.hasConsent
    }

    /**
     * Sets the in-memory consent (persisting it is the analytics store's job).
     * Withdrawing consent also throws away anything still queued, so events
     * recorded just before the user said no are never sent.
     */
    setConsent(value: AnalyticsConsent | null): void {
        _consent = value
        if (value !== 'granted') {
            this.queue = []
            this.pendingUserProps = null
            if (this.flushTimer !== null) { clearTimeout(this.flushTimer); this.flushTimer = null }
        }
    }

    /** Stored user properties are merged into the next track() call's data. */
    setUserProperties(props: Record<string, string>): void {
        if (!this.hasConsent || !WEBSITE_ID) {return}
        this.pendingUserProps = props
    }

    /** Send a pageview — populates Umami Overview (visits, sessions, bounce rate) */
    trackPageview(url: string, title: string = url): void {
        if (!this.hasConsent || !WEBSITE_ID) {return}
        fetch(ENDPOINT, {
            method:  'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({
                type: 'event',
                payload: {
                    website:  WEBSITE_ID,
                    hostname: 'desktop.librarium.app',
                    language: navigator.language,
                    referrer: '',
                    screen:   `${screen.width}x${screen.height}`,
                    title,
                    url:      `https://desktop.librarium.app${url}`,
                },
            }),
        }).catch(() => {})
    }

    /** Queue an analytics event */
    track(name: string, params: EventData = {}): void {
        if (!this.hasConsent || !WEBSITE_ID) {return}
        const data: EventData = { ...params }
        // A device model is the only free-form value events carry; make sure
        // it can never be a serial number.
        if (typeof data.model === 'string') { data.model = safeDeviceModel(data.model) }
        if (this.pendingUserProps) {
            Object.assign(data, this.pendingUserProps)
            this.pendingUserProps = null
        }
        this.queue.push({ name, data })
        this.scheduleFlush()
    }

    private scheduleFlush(): void {
        if (this.flushTimer !== null) {return}
        this.flushTimer = setTimeout(() => { this.doFlush(); this.flushTimer = null }, 2000)
    }

    private doFlush(): void {
        if (!this.hasConsent) { this.queue = []; return }
        for (const event of this.queue.splice(0)) {
            this.sendAsync(event)
        }
    }

    private buildPayload(event: UmamiEvent): string {
        return JSON.stringify({
            type: 'event',
            payload: {
                website:  WEBSITE_ID,
                hostname: 'desktop.librarium.app',
                language: navigator.language,
                referrer: '',
                screen:   '',
                title:    event.name,
                url:      `/${event.name}`,
                name:     event.name,
                data:     event.data,
            },
        })
    }

    private sendAsync(event: UmamiEvent): void {
        fetch(ENDPOINT, {
            method:  'POST',
            headers: { 'Content-Type': 'application/json' },
            body:    this.buildPayload(event),
        }).catch(() => {})
    }

    /** Awaitable flush via fetch — use when delivery must be confirmed before acting (e.g. opt-out) */
    async flush(): Promise<void> {
        if (this.flushTimer !== null) { clearTimeout(this.flushTimer); this.flushTimer = null }
        if (!this.hasConsent || this.queue.length === 0) {return}
        await Promise.all(
            this.queue.splice(0).map(event =>
                fetch(ENDPOINT, {
                    method:  'POST',
                    headers: { 'Content-Type': 'application/json' },
                    body:    this.buildPayload(event),
                }).catch(() => {})
            )
        )
    }

    /** Reliable sync flush via sendBeacon — call on app close only */
    flushSync(): void {
        if (this.flushTimer !== null) { clearTimeout(this.flushTimer); this.flushTimer = null }
        if (!this.hasConsent || this.queue.length === 0) {return}
        for (const event of this.queue.splice(0)) {
            navigator.sendBeacon(ENDPOINT, new Blob([this.buildPayload(event)], { type: 'application/json' }))
        }
    }

    trackSessionEnd(): void {
        if (!this.hasConsent) {return}
        const minutes = (Date.now() - this.sessionStart) / 60000
        this.track('app_close', { session_duration_bucket: bucketMinutes(minutes) })
        this.flushSync()
    }

    // ── Bucketing helpers (coarse values prevent fingerprinting) ──────────────
    // Library sizes, scan durations and search results have their own ranges
    // here; every other count or duration goes through `bucketCount` /
    // `bucketMinutes` (./privacy.ts).

    static bucketBookCount(n: number): string {
        if (n < 10)  {return '<10'}
        if (n < 50)  {return '10-50'}
        if (n < 200) {return '50-200'}
        return '200+'
    }

    static bucketDurationMs(ms: number): string {
        if (ms < 1000)  {return '<1s'}
        if (ms < 5000)  {return '1-5s'}
        if (ms < 30000) {return '5-30s'}
        return '30s+'
    }

    static bucketResultCount(n: number): string {
        if (n === 0) {return '0'}
        if (n <= 5)  {return '1-5'}
        if (n <= 20) {return '6-20'}
        return '20+'
    }

    static getOs(): string {
        const ua = navigator.userAgent
        if (ua.includes('Mac'))  {return 'macos'}
        if (ua.includes('Win'))  {return 'windows'}
        return 'linux'
    }
}

export const analytics = new Analytics()
export { Analytics }
