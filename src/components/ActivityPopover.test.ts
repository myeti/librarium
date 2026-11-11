import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'

vi.mock('@tauri-apps/api/core', () => import('@/test/tauri'))
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(() => Promise.resolve(() => {})) }))
import { mockInvoke } from '@/test/tauri'
import { cleanup, mountUi, testPinia } from '@/test/mountUi'
import ActivityPopover from './ActivityPopover.vue'
import { useDeviceStore } from '@/stores/device'
import type { Book, TransferJob } from '@/types'

function book(title: string): Book {
    return {
        id: title, path: `/lib/${title}.epub`, filename: `${title}.epub`, format: 'epub', sizeBytes: 1,
        dateAdded: '2026-01-01', metadata: { title, author: 'A', categories: [] },
    }
}

function job(id: string, status: TransferJob['status'], extra: Partial<TransferJob> = {}): TransferJob {
    return {
        id, direction: 'to-device', book: book(`Book ${id}`), status, progress: 40,
        needsConversion: true, targetFormat: 'azw3', ...extra,
    }
}

function mountWith(jobs: TransferJob[]) {
    testPinia()
    const device = useDeviceStore()
    device.transferJobs = jobs
    const wrapper = mountUi(ActivityPopover)
    return { wrapper, device, trigger: wrapper.find<HTMLButtonElement>('.activity__trigger') }
}

const popover = () => document.querySelector<HTMLElement>('.activity__popover')

beforeEach(() => mockInvoke.reset())
afterEach(() => cleanup())

describe('ActivityPopover', () => {
    it('says how many transfers run, and that one failed, in the item’s name', () => {
        const { trigger } = mountWith([job('1', 'transferring'), job('2', 'queued'), job('3', 'error')])
        expect(trigger.attributes('aria-label')).toBe('Activity, 2 transfers in progress, 1 transfer failed')
        expect(trigger.attributes('aria-expanded')).toBe('false')
    })

    it('opens a dialog listing each transfer with its state, progress and error', async () => {
        const { trigger } = mountWith([
            job('1', 'transferring'),
            job('2', 'error', { errorMessage: 'The e-reader is full.' }),
            { id: '3', direction: 'from-device', deviceTitle: 'Snow Crash', status: 'done', progress: 100, needsConversion: false, targetFormat: 'epub' },
        ])
        await trigger.trigger('click')
        await flushPromises()
        const dialog = popover()!
        expect(dialog.getAttribute('role')).toBe('dialog')
        expect(trigger.attributes('aria-expanded')).toBe('true')
        expect(document.activeElement).toBe(dialog)
        const rows = [...dialog.querySelectorAll('.activity__job')].map(r => r.textContent.replace(/\s+/g, ' ').trim())
        expect(rows[0]).toContain('Book 1')
        expect(rows[0]).toContain('Transferring… · EPUB → AZW3')
        expect(rows[1]).toContain('The e-reader is full.')
        expect(rows[2]).toContain('Snow Crash')
        expect(rows[2]).toContain('From the e-reader')
        expect(dialog.querySelectorAll('[role="progressbar"]')).toHaveLength(1)
    })

    it('cancels a job the backend can still stop', async () => {
        mockInvoke.on('cancel_transfer', () => undefined)
        const { trigger } = mountWith([job('1', 'queued'), job('2', 'transferring')])
        await trigger.trigger('click')
        await flushPromises()
        const cancels = popover()!.querySelectorAll<HTMLButtonElement>('button[aria-label^="Cancel transfer"]')
        // A copy that has started can't be stopped: no button for it.
        expect(cancels).toHaveLength(1)
        cancels[0].click()
        await flushPromises()
        expect(mockInvoke.callsTo('cancel_transfer').map(c => c.args)).toEqual([{ jobId: '1' }])
    })

    it('clears the finished jobs only (running ones stay followed)', async () => {
        const { trigger, device } = mountWith([job('1', 'done'), job('2', 'error'), job('3', 'cancelled'), job('4', 'transferring')])
        await trigger.trigger('click')
        await flushPromises()
        const clear = [...popover()!.querySelectorAll('button')].find(b => b.textContent?.includes('Clear finished'))!
        clear.click()
        await flushPromises()
        expect(device.transferJobs.map(j => j.id)).toEqual(['4'])
    })

    it('says when there is nothing, and closes with Escape (focus back on the item) or a click outside', async () => {
        const { trigger } = mountWith([])
        await trigger.trigger('click')
        await flushPromises()
        expect(popover()!.textContent).toContain('Nothing in progress')
        popover()!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }))
        await flushPromises()
        expect(popover()).toBeNull()
        expect(document.activeElement).toBe(trigger.element)

        await trigger.trigger('click')
        await flushPromises()
        document.body.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true }))
        await flushPromises()
        expect(popover()).toBeNull()
    })
})
