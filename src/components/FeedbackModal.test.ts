import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'

vi.mock('@tauri-apps/api/core', () => import('@/test/tauri'))
const sendFeedback = vi.fn<(message: string, device: unknown) => Promise<void>>()
// The real feedbackAttachments(): the dialog must show what is really sent.
vi.mock('@/services/librarium-api', async importOriginal => ({
    ...await importOriginal<typeof import('@/services/librarium-api')>(),
    sendFeedback: (m: string, d: unknown) => sendFeedback(m, d),
    reportError: vi.fn(),
}))
import FeedbackModal from './FeedbackModal.vue'
import { cacheAppVersion } from '@/services/librarium-api'
import { useDeviceStore } from '@/stores/device'
import { cleanup, keydown, mountUi, testPinia } from '@/test/mountUi'
import type { ConnectedDevice } from '@/types'

const dialogs = () => Array.from(document.querySelectorAll('dialog'))
const top = () => dialogs()[dialogs().length - 1]
const textarea = () => top().querySelector('textarea')!
const sendButton = () => Array.from(top().querySelectorAll('button')).find(b => b.textContent?.includes('Send'))!

function type(value: string) {
    textarea().value = value
    textarea().dispatchEvent(new Event('input'))
}

describe('FeedbackModal', () => {
    beforeEach(() => { sendFeedback.mockReset() })
    afterEach(cleanup)

    it('opens labelled, with the focus in its labelled message field', () => {
        mountUi(FeedbackModal)
        expect(document.getElementById(top().getAttribute('aria-labelledby')!)?.textContent).toContain('Send feedback')
        expect(document.activeElement).toBe(textarea())
        expect(document.querySelector(`label[for="${textarea().id}"]`)?.textContent).toContain('Your message')
        expect(document.getElementById(textarea().getAttribute('aria-describedby')!)?.textContent).toMatch(/Enter to send/)
        expect(sendButton().disabled).toBe(true)
    })

    it('closes on Escape while empty, but asks before throwing away a written message', async () => {
        const w = mountUi(FeedbackModal)
        type('The sync is great')
        await flushPromises()
        keydown(top(), 'Escape')
        await flushPromises()
        expect(w.emitted('close')).toBeUndefined()
        expect(top().textContent).toContain('Discard your changes?')
        keydown(top(), 'Escape')          // closes the question only
        await flushPromises()
        expect(dialogs()).toHaveLength(1)
        expect(textarea().value).toBe('The sync is great')

        type('   ')
        await flushPromises()
        keydown(top(), 'Escape')
        expect(w.emitted('close')).toHaveLength(1)
    })

    it('keeps the dialog and the message when sending fails', async () => {
        sendFeedback.mockRejectedValue(new Error('Server error 502'))
        const w = mountUi(FeedbackModal)
        type('Please add OPDS search')
        await flushPromises()
        sendButton().click()
        await flushPromises()
        expect(top().querySelector('[role="alert"]')!.textContent).toContain('Failed to send')
        expect(textarea().value).toBe('Please add OPDS search')
        expect(w.emitted('close')).toBeUndefined()
    })

    it('sends with ⌘/Ctrl+Enter and thanks the user', async () => {
        sendFeedback.mockResolvedValue()
        mountUi(FeedbackModal)
        type('  Lovely app  ')
        await flushPromises()
        keydown(textarea(), 'Enter', { ctrlKey: true })
        await flushPromises()
        expect(sendFeedback).toHaveBeenCalledWith('Lovely app', null)
        expect(top().textContent).toContain('Message sent')
    })

    describe('says what is sent with the message', () => {
        function connect(model: string) {
            const device: ConnectedDevice = {
                id: 'dev-1', name: 'KOBOeReader', mountPath: '/Volumes/KOBOeReader', deviceType: 'kobo',
                connection: 'massStorage', preferredFormat: 'epub', model,
            }
            const store = useDeviceStore(testPinia())
            store.devices = [device]
            store.selectedId = 'dev-1'
        }

        it('lists the version, the system, the e-reader and the installation ID', () => {
            cacheAppVersion('0.6.0')
            connect('Kobo Clara HD')
            mountUi(FeedbackModal)
            const items = Array.from(top().querySelectorAll('.feedback-attached li')).map(li => li.textContent?.trim())
            expect(top().querySelector('.feedback-attached')?.textContent).toContain('Sent with your message:')
            expect(items[0]).toBe('the app version (0.6.0)')
            expect(items[1]).toMatch(/^your operating system \((macOS|Windows|Linux)/)
            expect(items[2]).toBe('the connected e-reader (Kobo Clara HD)')
            expect(items[3]).toContain('random installation ID')
        })

        it('leaves the e-reader out when none is connected', () => {
            mountUi(FeedbackModal)
            const text = top().querySelector('.feedback-attached')?.textContent ?? ''
            expect(text).not.toContain('e-reader')
            expect(text).toContain('installation ID')
        })

        it('never shows nor sends a serial number as the model', async () => {
            sendFeedback.mockResolvedValue()
            connect('Kobo N418170012345')
            mountUi(FeedbackModal)
            expect(top().textContent).not.toContain('N418170012345')
            expect(top().querySelector('.feedback-attached')?.textContent).toContain('the connected e-reader (Kobo)')
            type('Hello')
            await flushPromises()
            sendButton().click()
            await flushPromises()
            // sendFeedback() applies safeDeviceModel itself (see consent.test.ts); the dialog passes the device on.
            expect(sendFeedback).toHaveBeenCalledWith('Hello', { brand: 'kobo', model: 'Kobo N418170012345' })
        })
    })
})
