import { defineStore } from 'pinia'

export type NotificationType = 'error' | 'warning' | 'success' | 'info'

/**
 * A button on a toast ("Undo", "Retry", "Show"). Clicking it dismisses the
 * toast, then calls `run`; `run` handles its own failures (a rejection is
 * only logged).
 */
export interface NotificationAction {
    label: string
    run: () => unknown
}

export interface AppNotification {
    id: string
    type: NotificationType
    message: string
    action: NotificationAction | null
    /** How long it stays up, not counting the time it was paused. */
    durationMs: number
}

/** What `push` accepts besides the type and message (a number is the duration, as before). */
export interface NotifyOptions {
    durationMs?: number
    action?: NotificationAction
}

/**
 * How long each kind of toast stays up when the caller doesn't say. Errors
 * and warnings usually carry a reason the user needs time to read, so they
 * stay longer than confirmations.
 */
export const DEFAULT_DURATION_MS: Record<NotificationType, number> = {
    error:   12_000,
    warning:  8_000,
    success:  4_000,
    info:     5_000,
}

/** A toast with a button stays at least this long: time to read it and reach the button. */
export const ACTION_MIN_DURATION_MS = 10_000

/** At most this many toasts at once; beyond, the oldest one that isn't an error goes. */
export const MAX_NOTIFICATIONS = 5

interface NotificationsState {
    items: AppNotification[]
    /** Timers are stopped (pointer or focus on the toasts, window hidden). */
    paused: boolean
}

/**
 * The countdown of each toast on screen. Kept out of the state: timer
 * handles aren't data to render. Ids are random UUIDs, so one map serves
 * every store instance (tests create several).
 */
interface Countdown {
    remaining: number
    startedAt: number
    handle: ReturnType<typeof setTimeout> | null
}
const countdowns = new Map<string, Countdown>()

/**
 * Toasts: the one place where the app tells the user what happened outside
 * a dialog (a failure, a confirmation, an "Undo"). Shown by
 * AppNotifications, above everything, dialogs included.
 *
 *     notifications.push('success', t('library.trashDone', { n }))
 *     notifications.push('error', t('errors.importFailed', { error }), {
 *         action: { label: t('library.retry'), run: () => this.importFiles() },
 *     })
 *
 * - Each toast disappears after its duration (by type, see
 *   DEFAULT_DURATION_MS; at least ACTION_MIN_DURATION_MS with a button), or
 *   when dismissed. `push` returns its id, for `dismiss`.
 * - Timers stop while the pointer or the focus is on the toasts (`pause` /
 *   `resume`, called by AppNotifications), so nothing vanishes while being
 *   read or clicked.
 * - The same message already on screen (a retry that failed again) isn't
 *   stacked twice: its timer starts over.
 * - Errors in a dialog are shown in the dialog, next to what failed; toasts
 *   are for what has no other place.
 */
export const useNotificationsStore = defineStore('notifications', {
    state: (): NotificationsState => ({
        items: [],
        paused: false,
    }),

    actions: {
        /** Shows a toast; returns its id. `options` may be the duration in ms. */
        push(type: NotificationType, message: string, options: number | NotifyOptions = {}): string {
            const opts: NotifyOptions = typeof options === 'number' ? { durationMs: options } : options
            const action = opts.action ?? null
            let durationMs = opts.durationMs ?? DEFAULT_DURATION_MS[type]
            if (action) { durationMs = Math.max(durationMs, ACTION_MIN_DURATION_MS) }

            const same = action ? undefined : this.items.find(n => n.type === type && n.message === message && !n.action)
            if (same) {
                this.startCountdown(same.id, Math.max(durationMs, same.durationMs))
                return same.id
            }

            const id = crypto.randomUUID()
            this.items.push({ id, type, message, action, durationMs })
            this.dropOverflow()
            this.startCountdown(id, durationMs)
            return id
        },

        dismiss(id: string) {
            const countdown = countdowns.get(id)
            if (countdown?.handle) { clearTimeout(countdown.handle) }
            countdowns.delete(id)
            this.items = this.items.filter(n => n.id !== id)
        },

        /** The toast's button was clicked: dismiss it, then run its action. */
        async runAction(id: string) {
            const action = this.items.find(n => n.id === id)?.action
            this.dismiss(id)
            if (!action) { return }
            try {
                await action.run()
            } catch (e) {
                console.error('[notifications] toast action failed', e)
            }
        },

        /** Stops every toast's timer (the user is reading or using them). */
        pause() {
            if (this.paused) { return }
            this.paused = true
            const now = Date.now()
            for (const countdown of countdowns.values()) {
                if (!countdown.handle) { continue }
                clearTimeout(countdown.handle)
                countdown.handle = null
                countdown.remaining = Math.max(0, countdown.remaining - (now - countdown.startedAt))
            }
        },

        /** Starts the timers again, each with the time it had left. */
        resume() {
            if (!this.paused) { return }
            this.paused = false
            for (const [id, countdown] of countdowns) {
                this.schedule(id, countdown)
            }
        },

        // ── Internal ──────────────────────────────────────────────────────

        startCountdown(id: string, durationMs: number) {
            const previous = countdowns.get(id)
            if (previous?.handle) { clearTimeout(previous.handle) }
            const countdown: Countdown = { remaining: durationMs, startedAt: Date.now(), handle: null }
            countdowns.set(id, countdown)
            if (!this.paused) { this.schedule(id, countdown) }
        },

        schedule(id: string, countdown: Countdown) {
            countdown.startedAt = Date.now()
            countdown.handle = setTimeout(() => this.dismiss(id), countdown.remaining)
        },

        /** Keeps at most MAX_NOTIFICATIONS: the oldest non-error goes first (errors matter most). */
        dropOverflow() {
            while (this.items.length > MAX_NOTIFICATIONS) {
                const victim = this.items.find(n => n.type !== 'error') ?? this.items[0]
                this.dismiss(victim.id)
            }
        },
    },
})
