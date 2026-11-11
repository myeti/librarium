import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'

vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(() => Promise.resolve(() => {})) }))
import {
    ACTION_MIN_DURATION_MS, DEFAULT_DURATION_MS, MAX_NOTIFICATIONS, useNotificationsStore,
} from './notifications'
import { formatArgs, MAX_LOG_ENTRIES, useLogStore } from './log'

beforeEach(() => {
    setActivePinia(createPinia())
    vi.useFakeTimers()
})
afterEach(() => { vi.useRealTimers() })

describe('notifications store', () => {
    it('shows a toast for its default duration, or the one asked for', () => {
        const store = useNotificationsStore()
        store.push('success', 'Saved')
        store.push('error', 'Disk full', 20_000)
        expect(store.items.map(n => n.message)).toEqual(['Saved', 'Disk full'])
        vi.advanceTimersByTime(DEFAULT_DURATION_MS.success)
        expect(store.items.map(n => n.message)).toEqual(['Disk full'])
        vi.advanceTimersByTime(20_000)
        expect(store.items).toEqual([])
    })

    it('keeps errors up longer than confirmations', () => {
        expect(DEFAULT_DURATION_MS.error).toBeGreaterThan(DEFAULT_DURATION_MS.warning)
        expect(DEFAULT_DURATION_MS.warning).toBeGreaterThan(DEFAULT_DURATION_MS.info)
        expect(DEFAULT_DURATION_MS.info).toBeGreaterThan(DEFAULT_DURATION_MS.success)
    })

    it('dismisses one toast on demand, by the id push returned', () => {
        const store = useNotificationsStore()
        const a = store.push('info', 'a')
        store.push('info', 'b')
        store.dismiss(a)
        expect(store.items.map(n => n.message)).toEqual(['b'])
    })

    it('runs a toast action once, after dismissing it; a failing action is only logged', async () => {
        const store = useNotificationsStore()
        const undo = vi.fn()
        const id = store.push('success', 'Moved to the Trash', { action: { label: 'Undo', run: undo } })
        await store.runAction(id)
        expect(undo).toHaveBeenCalledTimes(1)
        expect(store.items).toEqual([])
        await store.runAction(id)
        expect(undo).toHaveBeenCalledTimes(1)

        const error = vi.spyOn(console, 'error').mockImplementation(() => {})
        const retry = store.push('error', 'Failed', { action: { label: 'Retry', run: () => Promise.reject(new Error('again')) } })
        await expect(store.runAction(retry)).resolves.toBeUndefined()
        expect(error).toHaveBeenCalled()
    })

    it('leaves time to reach a toast’s button', () => {
        const store = useNotificationsStore()
        store.push('success', 'Removed', { action: { label: 'Undo', run: () => {} } })
        expect(store.items[0].durationMs).toBe(ACTION_MIN_DURATION_MS)
        vi.advanceTimersByTime(DEFAULT_DURATION_MS.success)
        expect(store.items).toHaveLength(1)
        vi.advanceTimersByTime(ACTION_MIN_DURATION_MS)
        expect(store.items).toHaveLength(0)
    })

    it('stops every timer while paused, and resumes each with the time it had left', () => {
        const store = useNotificationsStore()
        store.push('success', 'Saved')                     // 4 s
        vi.advanceTimersByTime(3_000)
        store.pause()
        store.push('info', 'Pushed while paused')          // starts on resume
        vi.advanceTimersByTime(60_000)
        expect(store.items.map(n => n.message)).toEqual(['Saved', 'Pushed while paused'])
        store.resume()
        vi.advanceTimersByTime(999)
        expect(store.items).toHaveLength(2)
        vi.advanceTimersByTime(1)
        expect(store.items.map(n => n.message)).toEqual(['Pushed while paused'])
        vi.advanceTimersByTime(DEFAULT_DURATION_MS.info)
        expect(store.items).toEqual([])
    })

    it('shows the same message once, its timer starting over', () => {
        const store = useNotificationsStore()
        const first = store.push('error', 'NAS offline')
        vi.advanceTimersByTime(DEFAULT_DURATION_MS.error - 1)
        expect(store.push('error', 'NAS offline')).toBe(first)
        expect(store.items).toHaveLength(1)
        vi.advanceTimersByTime(DEFAULT_DURATION_MS.error - 1)
        expect(store.items).toHaveLength(1)
        vi.advanceTimersByTime(1)
        expect(store.items).toHaveLength(0)
    })

    it('keeps at most MAX_NOTIFICATIONS, dropping the oldest that isn’t an error', () => {
        const store = useNotificationsStore()
        store.push('error', 'error 0')
        for (let i = 1; i <= MAX_NOTIFICATIONS; i++) { store.push('success', `ok ${i}`) }
        expect(store.items).toHaveLength(MAX_NOTIFICATIONS)
        expect(store.items[0].message).toBe('error 0')
        expect(store.items.map(n => n.message)).not.toContain('ok 1')
    })
})

describe('log store', () => {
    it('redacts secrets and keeps the newest MAX_LOG_ENTRIES lines', () => {
        const store = useLogStore()
        store.push('info', 'GET https://www.googleapis.com/books/v1/volumes?q=x&key=AIzaSECRET', 'frontend')
        expect(store.entries[0].message).not.toContain('AIzaSECRET')
        for (let i = 0; i < MAX_LOG_ENTRIES + 5; i++) { store.push('info', `line ${i}`, 'rust') }
        expect(store.entries).toHaveLength(MAX_LOG_ENTRIES)
        expect(store.entries[store.entries.length - 1].message).toBe(`line ${MAX_LOG_ENTRIES + 4}`)
    })

    it('writes console arguments readably', () => {
        expect(formatArgs(['[tidyup] result', { title: 'Dune' }, 3])).toBe('[tidyup] result {"title":"Dune"} 3')
        expect(formatArgs([new Error('boom')])).toBe('Error: boom')
        expect(formatArgs(['x'.repeat(5000)])).toHaveLength(4001)
    })
})
