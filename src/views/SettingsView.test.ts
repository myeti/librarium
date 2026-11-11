import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { enableAutoUnmount, flushPromises, mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { createMemoryHistory, createRouter } from 'vue-router'

vi.mock('@tauri-apps/api/core', () => import('@/test/tauri'))
vi.mock('@/utils/sounds', () => ({ playNewUser: vi.fn(), playIntro: vi.fn(), playTransferComplete: vi.fn() }))
import { backendError, mockInvoke } from '@/test/tauri'
import { stubMatchMedia } from '@/test/matchMedia'
import SettingsView from './SettingsView.vue'
import { useDeviceStore } from '@/stores/device'
import { useNotificationsStore } from '@/stores/notifications'
import { useSettingsStore } from '@/stores/settings'
import { useOpdsSourcesStore } from '@/stores/opdsSources'
import { useThemeStore } from '@/stores/theme'
import { REDUCED_MOTION_QUERY } from '@/stores/system'
import { i18nPlugin } from '@/i18n'
import type { AppConfig } from '@/services/appConfig'

/** Settings at `/settings<hash>`, with the settings read from `config`. */
async function mountSettings(hash = '', config: AppConfig = {}) {
    const pinia = createPinia()
    setActivePinia(pinia)
    useSettingsStore().initFromConfig(config)
    useOpdsSourcesStore().initFromConfig(config.opdsCatalogs ?? [])
    const router = createRouter({
        history: createMemoryHistory(),
        routes: [
            { path: '/settings', component: SettingsView },
            { path: '/:other(.*)*', component: { template: '<div />' } },
        ],
    })
    await router.push(`/settings${hash}`)
    const wrapper = mount({ template: '<RouterView />' }, {
        attachTo: document.body,
        global: { plugins: [pinia, i18nPlugin, router], stubs: { 'vue-feather': true } },
    })
    await flushPromises()
    return { wrapper, router }
}

/** The ConfirmDialog on screen (rendered in a native <dialog>). */
function confirmDialog() {
    const dialog = document.querySelector<HTMLElement>('[role="dialog"]')
    const buttons = [...(dialog?.querySelectorAll('button') ?? [])]
    // The last two: Cancel, then the action.
    return { dialog, cancel: buttons[buttons.length - 2], confirm: buttons[buttons.length - 1] }
}

enableAutoUnmount(afterEach)
beforeEach(() => {
    mockInvoke.reset()
    mockInvoke.on('opds_status', () => null)
    mockInvoke.on('set_config', () => undefined)
    stubMatchMedia({})
    vi.spyOn(console, 'error').mockImplementation(() => {})
})
afterEach(() => {
    vi.unstubAllGlobals()
    document.body.innerHTML = ''
})

describe('Settings: navigation', () => {
    it('lists the sections, shows the first one, and marks it as the current page', async () => {
        const { wrapper } = await mountSettings()
        const links = wrapper.findAll('.settings-nav__item')
        expect(links.map(l => l.text())).toEqual([
            'Library', 'Devices', 'Metadata', 'OPDS server', 'Catalogs', 'Appearance', 'Privacy & analytics', 'Updates', 'Backup', 'Advanced',
        ])
        expect(links[0].attributes('aria-current')).toBe('page')
        expect(wrapper.find('h2').text()).toBe('Library')
    })

    it('opens the section the address names, and follows the navigation', async () => {
        const { wrapper, router } = await mountSettings('#privacy')
        expect(wrapper.find('h2').text()).toBe('Privacy & analytics')
        await router.replace('/settings#appearance')
        await flushPromises()
        expect(wrapper.find('h2').text()).toBe('Appearance')
        expect(wrapper.find('.settings-nav__item[aria-current="page"]').text()).toBe('Appearance')
    })

    it('opens Devices and scrolls to the MTP setting from the sidebar’s link (#mtp-setting)', async () => {
        const scrolled: string[] = []
        vi.spyOn(Element.prototype, 'scrollIntoView').mockImplementation(function (this: Element) { scrolled.push(this.id) })
        const { wrapper } = await mountSettings('#mtp-setting')
        expect(wrapper.find('h2').text()).toBe('Devices')
        expect(scrolled).toEqual(['mtp-setting'])
    })
})

describe('Settings › Devices: MTP Kindles', () => {
    async function mtpCheckbox(config: AppConfig = {}) {
        const mounted = await mountSettings('#devices', config)
        return { ...mounted, checkbox: mounted.wrapper.find<HTMLInputElement>('#mtp-setting input[type="checkbox"]') }
    }

    it('shows the saved setting (on unless it is `false`)', async () => {
        expect((await mtpCheckbox({ mtpEnabled: false })).checkbox.element.checked).toBe(false)
        expect((await mtpCheckbox({})).checkbox.element.checked).toBe(true)
        expect((await mtpCheckbox({ mtpEnabled: 'yes' } as unknown as AppConfig)).checkbox.element.checked).toBe(true)
    })

    it('explains what MTP Kindles are, and what can hold them', async () => {
        const card = (await mtpCheckbox()).wrapper.find('#mtp-setting')
        expect(card.text()).toContain('MTP Kindles (experimental)')
        expect(card.text()).toContain('Image Capture')
        expect(card.text()).toContain('70-librarium-kindle.rules')
    })

    it('saves the setting, then polls at once to show what the Kindle does', async () => {
        const { checkbox } = await mtpCheckbox({ mtpEnabled: false })
        mockInvoke.on('detect_devices', () => [])
        mockInvoke.on('get_mtp_status', () => ({
            enabled: true, devices: [{ mountPath: 'mtp://usb-20-1.2', name: 'Kindle', state: 'connecting' }],
        }))
        await checkbox.setValue(true)
        await flushPromises()
        expect(mockInvoke.callsTo('set_config').map(c => c.args)).toEqual([{ updates: { mtpEnabled: true } }])
        expect(useSettingsStore().mtpEnabled).toBe(true)
        expect(useDeviceStore().mtpNotices.map(n => n.state)).toEqual(['connecting'])
    })

    it('puts the switch back and says why when it can’t be saved', async () => {
        const { checkbox } = await mtpCheckbox({ mtpEnabled: false })
        mockInvoke.on('set_config', () => backendError({ code: 'IO', message: 'the settings file is damaged' }))
        await checkbox.setValue(true)
        await flushPromises()
        expect(useSettingsStore().mtpEnabled).toBe(false)
        expect(checkbox.element.checked).toBe(false)
        const items = useNotificationsStore().items
        expect(items[items.length - 1]).toMatchObject({
            type: 'error', message: 'Could not change MTP support: the settings file is damaged',
        })
        expect(mockInvoke.callsTo('detect_devices')).toHaveLength(0)
    })
})

describe('Settings: save feedback', () => {
    it('saves the e-reader folder when the field is left, and says so', async () => {
        const { wrapper } = await mountSettings('#devices')
        const field = wrapper.find('#device-subfolder')
        await field.setValue('  Mes livres ')
        await field.trigger('blur')
        await flushPromises()
        expect(mockInvoke.callsTo('set_config').map(c => c.args)).toEqual([{ updates: { deviceSubfolder: 'Mes livres' } }])
        expect(wrapper.find('.save-status').text()).toBe('Saved')
    })

    it('says when a setting was not saved, and why', async () => {
        const { wrapper } = await mountSettings('#devices')
        mockInvoke.on('set_config', () => backendError({ code: 'IO', message: 'read-only volume' }))
        const field = wrapper.find('#device-subfolder')
        await field.setValue('Books')
        await field.trigger('keydown', { key: 'Enter' })
        await flushPromises()
        expect(wrapper.find('.save-status').text()).toBe('Not saved: read-only volume')
    })
})

describe('Settings › OPDS server: port', () => {
    it('refuses a port that isn’t one (no NaN saved), and saves a good one', async () => {
        localStorage.removeItem('opdsPort')
        const { wrapper } = await mountSettings('#opds')
        const port = wrapper.find('#opds-port')
        await port.setValue('80a')
        await port.trigger('blur')
        expect(wrapper.text()).toContain('Enter a port between 1024 and 65535.')
        expect(port.attributes('aria-invalid')).toBe('true')
        expect(localStorage.getItem('opdsPort')).toBeNull()

        await port.setValue('9000')
        await port.trigger('blur')
        await flushPromises()
        expect(wrapper.text()).not.toContain('Enter a port between')
        expect(localStorage.getItem('opdsPort')).toBe('9000')
        localStorage.removeItem('opdsPort')
    })
})

describe('Settings › Catalogs', () => {
    const catalog = { id: 'c1', url: 'https://books.example.org/opds', name: 'My shelf', apiKey: '' }

    it('asks before removing a catalog', async () => {
        const { wrapper } = await mountSettings('#catalogs', { opdsCatalogs: [catalog] })
        await wrapper.find('button[aria-label="Remove My shelf"]').trigger('click')
        await flushPromises()
        expect(confirmDialog().dialog?.textContent).toContain('Remove the catalog “My shelf”?')
        confirmDialog().cancel.click()
        await flushPromises()
        expect(useOpdsSourcesStore().sources).toHaveLength(1)

        await wrapper.find('button[aria-label="Remove My shelf"]').trigger('click')
        await flushPromises()
        confirmDialog().confirm.click()
        await flushPromises()
        expect(useOpdsSourcesStore().sources).toEqual([])
        expect(mockInvoke.callsTo('set_config').map(c => c.args)).toEqual([{ updates: { opdsCatalogs: [] } }])
    })

    it('only adds an http(s) address', async () => {
        const { wrapper } = await mountSettings('#catalogs')
        const url = wrapper.find('input[type="url"]')
        await url.setValue('ftp://nope')
        await wrapper.find('form').trigger('submit')
        expect(wrapper.text()).toContain('Enter an address starting with http:// or https://')
        expect(useOpdsSourcesStore().sources).toEqual([])

        await url.setValue('https://books.example.org/opds')
        await wrapper.find('form').trigger('submit')
        await flushPromises()
        expect(useOpdsSourcesStore().sources.map(s => s.url)).toEqual(['https://books.example.org/opds'])
    })
})

describe('Settings › Library: folder', () => {
    it('asks before going back to the default folder', async () => {
        mockInvoke.on('check_path_accessible', () => true)
        mockInvoke.on('plugin:path|resolve_directory', () => '/home/me/Documents')
        mockInvoke.on('plugin:path|join', ({ paths }) => (paths as string[]).join('/'))
        mockInvoke.on('list_users', () => [])
        const { wrapper } = await mountSettings('#library', { libraryRootOverride: '/Volumes/books' })
        const reset = wrapper.findAll('button').find(b => b.text() === 'Use the default folder')!
        await reset.trigger('click')
        await flushPromises()
        expect(confirmDialog().dialog?.textContent).toContain('/Volumes/books')
        expect(mockInvoke.callsTo('set_config')).toHaveLength(0)

        confirmDialog().confirm.click()
        await flushPromises()
        expect(mockInvoke.callsTo('set_config').map(c => c.args)).toEqual([{ updates: { libraryRootOverride: null } }])
        expect(useSettingsStore().libraryRootOverride).toBeNull()
    })
})

describe('Settings › Appearance', () => {
    it('shows each theme as a title page in its own colours, and saves a choice', async () => {
        const { wrapper } = await mountSettings('#appearance')
        const editions = wrapper.findAll('.edition')
        expect(editions.map(e => e.find('.edition__name').text())).toEqual(['Nocturne', 'Saphir', 'Folklore', 'Cerulean'])
        // Each preview carries its theme ('' is Nocturne, the default).
        expect(editions.map(e => e.find('.edition__page').attributes('data-theme')))
            .toEqual(['', 'saphir', 'folklore', 'cerulean'])
        expect(editions[0].classes()).toContain('-checked')

        const folklore = wrapper.find<HTMLInputElement>('input[name="theme"][value="folklore"]')
        folklore.element.checked = true
        await folklore.trigger('change')
        await flushPromises()
        expect(useThemeStore().theme).toBe('folklore')
        expect(document.documentElement.getAttribute('data-theme')).toBe('folklore')
        // "Saved" shows beside the row's title, not beside the cards (it would push them aside).
        expect(wrapper.find('#appearance .ui-setting__heading .save-status').text()).toContain('Saved')
        expect(wrapper.find('.theme-picker').element.parentElement!.querySelector('.save-status')).toBeNull()
        expect(mockInvoke.callsTo('set_config').map(c => c.args)).toEqual([{ updates: { theme: 'folklore' } }])
    })

    it('shows the effects default, Off under "reduce motion", and why', async () => {
        stubMatchMedia({ [REDUCED_MOTION_QUERY]: true })
        const { wrapper } = await mountSettings('#appearance')
        expect(wrapper.find<HTMLInputElement>('input[type="radio"][value="off"]').element.checked).toBe(true)
        expect(wrapper.text()).toContain('Off by default: your system asks to reduce motion.')
    })

    it('keeps the launch sound off by default', async () => {
        const { wrapper } = await mountSettings('#appearance')
        const launch = wrapper.find<HTMLInputElement>('#launch-sound')
        expect(launch.element.checked).toBe(false)
        // Labelled by its row's title.
        expect(document.querySelector('label[for="launch-sound"]')?.textContent).toBe('Play a sound when Librarium starts')
    })
})

describe('Settings › Updates: the launch checks', () => {
    it('are both on by default, each labelled by its row', async () => {
        const { wrapper } = await mountSettings('#updates')
        expect(wrapper.find<HTMLInputElement>('#auto-check-updates').element.checked).toBe(true)
        expect(wrapper.find<HTMLInputElement>('#show-news').element.checked).toBe(true)
        expect(document.querySelector('label[for="auto-check-updates"]')?.textContent).toBe('Check for updates automatically')
        expect(document.querySelector('label[for="show-news"]')?.textContent).toBe('Show news from Librarium')
    })

    it('show the saved choice and save a change', async () => {
        const { wrapper } = await mountSettings('#updates', { showNews: false })
        expect(wrapper.find<HTMLInputElement>('#show-news').element.checked).toBe(false)
        await wrapper.find<HTMLInputElement>('#auto-check-updates').setValue(false)
        await flushPromises()
        expect(useSettingsStore().autoCheckUpdates).toBe(false)
        expect(mockInvoke.callsTo('set_config').map(c => c.args)).toEqual([{ updates: { autoCheckUpdates: false } }])
    })
})

describe('Settings › Privacy & analytics', () => {
    it('says what statistics send in the consent dialog’s words', async () => {
        const { wrapper } = await mountSettings('#privacy')
        const disclosure = wrapper.find('.analytics-disclosure')
        expect(disclosure.exists()).toBe(true)
        expect(disclosure.text()).toContain('theme, screen size, and e-reader brand and model')
        expect(disclosure.text()).toContain('random installation ID')
        expect(disclosure.text()).toContain('never the error message')
    })

    it('lists the other connections, with the launch checks as they are set', async () => {
        const { wrapper } = await mountSettings('#privacy', { showNews: false })
        expect(wrapper.text()).toContain('At each launch, Librarium asks GitHub whether a new version exists.')
        expect(wrapper.text()).toContain('Fetching the “What’s new” messages at launch is turned off.')
        const link = wrapper.find('a.privacy-link')
        expect(link.attributes('href')).toBe('/settings#updates')
        expect(link.text()).toContain('Settings › Updates')
    })

    it('opens the privacy policy', async () => {
        const { wrapper } = await mountSettings('#privacy')
        const open = wrapper.findAll('button').find(b => b.text() === 'Read the privacy policy')!
        await open.trigger('click')
        await flushPromises()
        const dialog = document.querySelector('dialog')!
        expect(dialog.textContent).toContain('At a glance')
        expect(dialog.textContent).not.toContain('TODO')
    })
})
