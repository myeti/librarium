import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { debounce } from './debounce'

beforeEach(() => { vi.useFakeTimers() })
afterEach(() => { vi.useRealTimers() })

describe('debounce', () => {
    it('calls once, after the pause, with the last arguments', () => {
        const fn = vi.fn()
        const d = debounce(fn, 150)
        d('d'); d('du'); d('dun')
        vi.advanceTimersByTime(149)
        expect(fn).not.toHaveBeenCalled()
        d('dune')
        vi.advanceTimersByTime(149)
        expect(fn).not.toHaveBeenCalled()
        vi.advanceTimersByTime(1)
        expect(fn).toHaveBeenCalledTimes(1)
        expect(fn).toHaveBeenCalledWith('dune')
    })

    it('cancel() drops the pending call', () => {
        const fn = vi.fn()
        const d = debounce(fn, 150)
        d('x')
        expect(d.pending).toBe(true)
        d.cancel()
        expect(d.pending).toBe(false)
        vi.runAllTimers()
        expect(fn).not.toHaveBeenCalled()
    })

    it('flush() runs the pending call now, and only once', () => {
        const fn = vi.fn()
        const d = debounce(fn, 150)
        d.flush() // nothing pending: nothing happens
        d('x')
        d.flush()
        expect(fn).toHaveBeenCalledWith('x')
        vi.runAllTimers()
        expect(fn).toHaveBeenCalledTimes(1)
    })
})
