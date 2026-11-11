import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { mount, type VueWrapper } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'

vi.mock('@tauri-apps/api/core', () => import('@/test/tauri'))
import MessagesModal from './MessagesModal.vue'
import { i18nPlugin } from '@/i18n'
import type { ApiMessage } from '@/services/librarium-api'

function message(content: string): ApiMessage {
    return {
        id: 1,
        translations: { en: { title: 'News', content }, fr: { title: 'Nouvelles', content } },
        constraintOs: null,
        constraintVersion: null,
        createdAt: '2026-11-05',
    }
}

describe('MessagesModal', () => {
    let wrapper: VueWrapper | null = null

    beforeEach(() => {
        delete (window as unknown as Record<string, unknown>).__xss
    })

    afterEach(() => {
        wrapper?.unmount()
        wrapper = null
        document.body.innerHTML = ''
    })

    function render(content: string) {
        const pinia = createPinia()
        setActivePinia(pinia)
        wrapper = mount(MessagesModal, {
            props: { messages: [message(content)] },
            global: { plugins: [pinia, i18nPlugin], stubs: { 'vue-feather': true } },
            attachTo: document.body,
        })
        // The modal is teleported to <body>.
        return document.body.querySelector('.message-card-body') as HTMLElement
    }

    it('keeps HTML from the server inert', () => {
        const payload = '<img src=x onerror="window.__xss=1"><script>window.__xss=2</script><a href="javascript:alert(1)">x</a>'
        const body = render(payload)
        expect(body.querySelector('img')).toBeNull()
        expect(body.querySelector('script')).toBeNull()
        expect(body.querySelector('a')).toBeNull()
        expect(body.textContent).toContain('<img src=x onerror=')
        expect((window as unknown as Record<string, unknown>).__xss).toBeUndefined()
    })

    it('renders paragraphs, line breaks and safe links', () => {
        const body = render('Hello\nworld\n\nRead https://librarium.app/notes now')
        const paragraphs = body.querySelectorAll('p')
        expect(paragraphs).toHaveLength(2)
        expect(paragraphs[0].querySelector('br')).not.toBeNull()
        const link = body.querySelector('a') as HTMLAnchorElement
        expect(link.getAttribute('href')).toBe('https://librarium.app/notes')
        expect(link.getAttribute('target')).toBe('_blank')
        expect(link.getAttribute('rel')).toContain('noopener')
    })

    it('is a labelled dialog with the focus on “Got it”, closed by Escape or a click outside', () => {
        render('Hello')
        const dialog = document.querySelector('dialog')!
        expect(document.getElementById(dialog.getAttribute('aria-labelledby')!)?.textContent).toContain('What’s new')
        expect(document.activeElement?.textContent).toContain('Got it')
        dialog.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }))
        expect(wrapper!.emitted('close')).toHaveLength(1)
        dialog.dispatchEvent(new MouseEvent('mousedown', { bubbles: true }))
        dialog.dispatchEvent(new MouseEvent('click', { bubbles: true }))
        expect(wrapper!.emitted('close')).toHaveLength(2)
    })
})
