import { afterEach, beforeEach, describe, expect, it, vi, type Mock } from 'vitest'
import { flushPromises } from '@vue/test-utils'

vi.mock('@tauri-apps/api/core', () => import('@/test/tauri'))
import AnalyticsConsentDialog from './AnalyticsConsentDialog.vue'
import { useAnalyticsStore } from '@/stores/analytics'
import { useModalsStore } from '@/stores/modals'
import { cleanup, keydown, mountUi, testPinia } from '@/test/mountUi'

describe('AnalyticsConsentDialog', () => {
    let setConsent: Mock<(value: string) => Promise<void>>

    beforeEach(() => {
        setConsent = vi.fn((_value: string) => Promise.resolve())
        const store = useAnalyticsStore(testPinia())
        store.setConsent = setConsent
    })

    afterEach(cleanup)

    function open() {
        mountUi(AnalyticsConsentDialog)
        const dialog = document.querySelector('dialog')!
        const button = (text: string) => Array.from(dialog.querySelectorAll('button')).find(b => b.textContent?.includes(text))!
        return { dialog, deny: button('No thanks'), grant: button('Allow'), policy: button('privacy policy') }
    }

    it('is a modal dialog labelled by its title and described by its intro', () => {
        const { dialog } = open()
        expect(dialog.open).toBe(true)
        expect(document.getElementById(dialog.getAttribute('aria-labelledby')!)?.textContent).toContain('Help improve Librarium?')
        expect(document.getElementById(dialog.getAttribute('aria-describedby')!)?.textContent).toContain('Nothing is sent unless you allow it')
        expect(useModalsStore().count).toBe(1)
    })

    it('starts on "No thanks" (the privacy-preserving answer)', () => {
        const { deny } = open()
        expect(document.activeElement).toBe(deny)
        expect(deny.textContent).toContain('No thanks')
    })

    it('records the answer clicked', async () => {
        const { grant } = open()
        grant.click()
        await flushPromises()
        expect(setConsent).toHaveBeenCalledWith('granted')
    })

    it('Escape means "No thanks"', async () => {
        const { dialog } = open()
        keydown(dialog, 'Escape')
        await flushPromises()
        expect(setConsent).toHaveBeenCalledWith('denied')
    })

    it('says what is sent with the same words as Settings, and that error messages never are', () => {
        const { dialog } = open()
        const text = dialog.textContent ?? ''
        expect(dialog.querySelector('.analytics-disclosure')).not.toBeNull()
        expect(text).toContain('rounded into ranges')
        expect(text).toContain('never the error message')
        expect(text).toContain('random installation ID')
        expect(text).toMatch(/Never sent:.*file names or paths, error messages/)
    })

    it('opens the privacy policy above itself; Escape closes the policy only', async () => {
        const { policy } = open()
        policy.click()
        await flushPromises()
        const dialogs = Array.from(document.querySelectorAll('dialog'))
        expect(dialogs).toHaveLength(2)
        expect(dialogs[1].textContent).toContain('Privacy policy')
        keydown(dialogs[1], 'Escape')
        await flushPromises()
        expect(document.querySelectorAll('dialog')).toHaveLength(1)
        expect(setConsent).not.toHaveBeenCalled()
    })

    it('answers only once while saving', async () => {
        let finish: () => void = () => {}
        setConsent.mockImplementation(() => new Promise<void>(r => { finish = r }))
        const { dialog, deny } = open()
        deny.click()
        keydown(dialog, 'Escape')
        expect(setConsent).toHaveBeenCalledOnce()
        finish()
        await flushPromises()
    })
})
