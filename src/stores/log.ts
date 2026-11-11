import { defineStore } from 'pinia'
import { listen } from '@tauri-apps/api/event'
import { redactSecrets } from '@/utils/redact'

export type LogLevel = 'info' | 'warn' | 'error'

export interface LogEntry {
    id: number
    time: string
    level: LogLevel
    message: string
    source: 'frontend' | 'rust'
}

/** Older entries are dropped beyond this many. */
export const MAX_LOG_ENTRIES = 500

function now(): string {
    return new Date().toISOString().substring(11, 23) // HH:mm:ss.mmm
}

/** A log line is cut beyond this many characters (someone logging a whole book list…). */
const MAX_LINE_LENGTH = 4000

/** Joins console arguments into one line; objects as JSON when possible (`String()` gives "[object Object]"). */
export function formatArgs(args: unknown[]): string {
    const line = args.map(a => {
        if (typeof a === 'string') { return a }
        if (a instanceof Error) { return `${a.name}: ${a.message}` }
        if (a !== null && typeof a === 'object') {
            try { return JSON.stringify(a) } catch { return Object.prototype.toString.call(a) }
        }
        return String(a)
    }).join(' ')
    return line.length > MAX_LINE_LENGTH ? `${line.slice(0, MAX_LINE_LENGTH)}…` : line
}

/**
 * The in-app debug log (shown by DebugPanel, Ctrl+Shift+D): the frontend's
 * console output and the backend's `app:log` events, newest last, capped at
 * MAX_LOG_ENTRIES. Every line is redacted (API keys in URLs) when it is
 * recorded, because the panel has "Copy all".
 */
export const useLogStore = defineStore('log', {
    state: () => ({
        entries: [] as LogEntry[],
        nextId: 0,
        installed: false,
    }),

    actions: {
        push(level: LogLevel, message: string, source: LogEntry['source']) {
            // Redacted here, not only where URLs are built: any frontend or
            // Rust log line may contain a URL with an API key in it.
            this.entries.push({ id: this.nextId++, time: now(), level, message: redactSecrets(message), source })
            if (this.entries.length > MAX_LOG_ENTRIES) { this.entries.shift() }
        },

        clear() {
            this.entries = []
        },

        /**
         * Starts recording: wraps `console.log/warn/error` (they still print)
         * and listens for the backend's log events. Once per app run.
         */
        install() {
            if (this.installed) { return }
            this.installed = true
            const orig = { log: console.log, warn: console.warn, error: console.error }
            console.log = (...args: unknown[]) => { orig.log(...args); this.push('info', formatArgs(args), 'frontend') }
            console.warn = (...args: unknown[]) => { orig.warn(...args); this.push('warn', formatArgs(args), 'frontend') }
            console.error = (...args: unknown[]) => { orig.error(...args); this.push('error', formatArgs(args), 'frontend') }

            listen<string>('app:log', event => {
                const msg = event.payload
                const level: LogLevel = msg.includes('ERROR') ? 'error' : msg.includes('WARN') ? 'warn' : 'info'
                this.push(level, msg, 'rust')
            }).catch((e: unknown) => {
                // Without the listener only the backend's lines are missing.
                orig.error('[log] could not listen for backend log events', e)
            })
        },
    },
})
