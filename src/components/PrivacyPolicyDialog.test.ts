import { afterEach, describe, expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'

vi.mock('@tauri-apps/api/core', () => import('@/test/tauri'))
import PrivacyPolicyDialog from './PrivacyPolicyDialog.vue'
import { currentLang } from '@/i18n'
import { cleanup, keydown, mountUi } from '@/test/mountUi'
import privacyPolicy from '../../PRIVACY.md?raw'

const dialog = () => document.querySelector('dialog')!

describe('PrivacyPolicyDialog', () => {
    afterEach(() => {
        cleanup()
        currentLang.value = 'en'
    })

    it('shows PRIVACY.md, titled by the dialog rather than the file’s own heading', () => {
        mountUi(PrivacyPolicyDialog)
        expect(document.getElementById(dialog().getAttribute('aria-labelledby')!)?.textContent).toContain('Privacy policy')
        expect(dialog().querySelector('h1')).toBeNull()
        const sections = Array.from(dialog().querySelectorAll('h3')).map(h => h.textContent?.trim())
        expect(sections).toContain('At a glance')
        expect(sections).toContain('Error reports')
        expect(sections).toContain('Updates')
        expect(dialog().querySelectorAll('table').length).toBeGreaterThan(2)
        expect(dialog().querySelector('th[scope="col"]')).not.toBeNull()
    })

    it('leaves out the HTML comments (the owner’s TODO stays in the file only)', () => {
        expect(privacyPolicy).toContain('<!--')
        mountUi(PrivacyPolicyDialog)
        const text = dialog().textContent ?? ''
        expect(text).not.toContain('TODO')
        expect(text).not.toContain('<!--')
    })

    it('renders no HTML from the document: tags show as text', () => {
        mountUi(PrivacyPolicyDialog, {
            props: { source: '## Test\n\n<img src=x onerror="alert(1)"> <script>alert(2)</script> **bold** [x](javascript:alert(3))' },
        })
        const body = dialog().querySelector('.privacy-policy')!
        expect(body.querySelector('img, script, iframe')).toBeNull()
        expect(body.textContent).toContain('<img src=x onerror="alert(1)">')
        expect(body.textContent).toContain('<script>alert(2)</script>')
        expect(body.querySelector('strong')?.textContent).toBe('bold')
        expect(body.querySelector('a')).toBeNull()
    })

    it('opens web links in the browser and follows links to its own sections in place', async () => {
        mountUi(PrivacyPolicyDialog)
        const external = Array.from(dialog().querySelectorAll<HTMLAnchorElement>('a[target="_blank"]'))
        expect(external.length).toBeGreaterThan(0)
        for (const a of external) {
            expect(a.href).toMatch(/^https?:/)
            expect(a.rel).toBe('noopener noreferrer')
        }
        const toErrors = Array.from(dialog().querySelectorAll<HTMLAnchorElement>('a[href="#error-reports"]'))[0]
        toErrors.click()
        await flushPromises()
        expect(document.activeElement?.textContent?.trim()).toBe('Error reports')
        expect(window.location.hash).toBe('')
    })

    it('says the policy is in English when the app is not', () => {
        currentLang.value = 'fr'
        mountUi(PrivacyPolicyDialog)
        expect(dialog().textContent).toContain('n’existe qu’en anglais')
    })

    it('closes on Escape and with its button', () => {
        const w = mountUi(PrivacyPolicyDialog)
        keydown(dialog(), 'Escape')
        Array.from(dialog().querySelectorAll('button')).find(b => b.textContent?.trim() === 'Close')!.click()
        expect(w.emitted('close')).toHaveLength(2)
    })
})
