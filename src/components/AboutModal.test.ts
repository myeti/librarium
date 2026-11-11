import { afterEach, describe, expect, it, vi } from 'vitest'

vi.mock('@tauri-apps/api/core', () => import('@/test/tauri'))
import AboutModal from './AboutModal.vue'
import { useModalsStore } from '@/stores/modals'
import { cleanup, keydown, mountUi, testPinia } from '@/test/mountUi'

describe('AboutModal', () => {
    afterEach(cleanup)

    it('is a named dialog in the modal stack, with a labelled close button', () => {
        mountUi(AboutModal)
        const dialog = document.querySelector('dialog')!
        expect(dialog.getAttribute('aria-label')).toBe('About Librarium')
        expect(useModalsStore(testPinia()).count).toBe(1)
        const close = dialog.querySelector<HTMLButtonElement>('.about-close')!
        expect(close.getAttribute('aria-label')).toBe('Close')
        expect(document.activeElement).toBe(close)
    })

    it('closes on Escape, the × and a click outside (nothing to lose)', () => {
        const w = mountUi(AboutModal)
        const dialog = document.querySelector('dialog')!
        keydown(dialog, 'Escape')
        dialog.querySelector<HTMLButtonElement>('.about-close')!.click()
        dialog.dispatchEvent(new MouseEvent('mousedown', { bubbles: true }))
        dialog.dispatchEvent(new MouseEvent('click', { bubbles: true }))
        expect(w.emitted('close')).toHaveLength(3)
    })

    it('lists the 2024 Kindles apart, as MTP and experimental (COMPATIBILITY.md)', () => {
        mountUi(AboutModal)
        const models = (selector: string) => Array.from(document.querySelectorAll(`${selector} .device-row-model`)).map(el => el.textContent?.trim())
        const usb = models('.device-list:not(.-mtp)')
        const mtp = models('.device-list.-mtp')
        expect(mtp).toEqual(['Paperwhite — 12th gen', 'Colorsoft', 'Kindle — 2024', 'Scribe — 2024'])
        for (const model of mtp) { expect(usb).not.toContain(model) }
        expect(usb.some(m => /Paperwhite 6|Colorsoft|12th/.test(m ?? ''))).toBe(false)
        expect(document.querySelector('.device-col-sub')?.textContent).toContain('MTP, experimental')
        expect(document.querySelector('.device-note')?.textContent).toContain('MTP Kindles (experimental)')
    })

    it('credits only what the app still ships', () => {
        mountUi(AboutModal)
        const credits = Array.from(document.querySelectorAll('.credit-name')).map(el => el.textContent?.trim())
        expect(credits).toContain('boko')
        expect(credits).not.toContain('epub (Rust)')
        expect(credits).not.toContain('mobi')
        expect(credits).not.toContain('epub-builder')
    })
})
