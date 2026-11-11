import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises, mount, RouterLinkStub } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'

vi.mock('@tauri-apps/api/core', () => import('@/test/tauri'))
vi.mock('@/utils/sounds', () => ({ playNewUser: vi.fn(), playIntro: vi.fn(), playTransferComplete: vi.fn() }))
import { mockInvoke } from '@/test/tauri'
import AppSidebar from './AppSidebar.vue'
import { useDeviceStore } from '@/stores/device'
import { currentLang, i18nPlugin } from '@/i18n'
import type { ConnectedDevice, MtpDevice, MtpDeviceState } from '@/types'

const paperwhite = (id: string, mountPath: string): ConnectedDevice => ({
    id, name: 'Kindle Paperwhite 5', mountPath, deviceType: 'kindle',
    connection: mountPath.startsWith('mtp://') ? 'mtp' : 'massStorage',
    preferredFormat: 'azw3', model: 'Kindle Paperwhite 5', freeSpaceBytes: 1, totalSpaceBytes: 4,
})

/** The sidebar with `devices` connected (the first one selected). */
function mountSidebar(devices: ConnectedDevice[]) {
    const pinia = createPinia()
    setActivePinia(pinia)
    const store = useDeviceStore()
    store.devices = devices
    store.selectedId = devices[0]?.id ?? null
    const wrapper = mount(AppSidebar, {
        global: { plugins: [pinia, i18nPlugin], stubs: { 'vue-feather': true, RouterLink: RouterLinkStub } },
    })
    return { store, wrapper }
}

function ejectButton(wrapper: ReturnType<typeof mountSidebar>['wrapper']) {
    return wrapper.find('button.device-eject')
}

beforeEach(() => mockInvoke.reset())

describe('AppSidebar devices', () => {
    it('offers a picker only with several devices, telling twins apart by volume', async () => {
        const { wrapper, store } = mountSidebar([paperwhite('a', '/Volumes/Kindle'), paperwhite('b', '/Volumes/Kindle 1')])
        const options = wrapper.findAll('select.device-select option')
        expect(options.map(o => o.text())).toEqual(['Kindle Paperwhite 5 (Kindle)', 'Kindle Paperwhite 5 (Kindle 1)'])

        mockInvoke.on('scan_device_library', () => [])
        mockInvoke.on('list_device_books', () => [])
        await wrapper.find('select.device-select').setValue('b')
        await flushPromises()
        expect(store.device?.id).toBe('b')
        expect(mockInvoke.callsTo('list_device_books')[0].args).toEqual({ devicePath: '/Volumes/Kindle 1' })

        const single = mountSidebar([paperwhite('a', '/Volumes/Kindle')])
        expect(single.wrapper.find('select.device-select').exists()).toBe(false)
    })

    it('ejects the selected device, and waits while a transfer to it runs', async () => {
        const { wrapper, store } = mountSidebar([paperwhite('a', '/Volumes/Kindle')])
        store.transferJobs.push({
            id: 'j', direction: 'to-device', status: 'transferring', progress: 40,
            needsConversion: false, targetFormat: 'azw3', devicePath: '/Volumes/Kindle',
        })
        await flushPromises()
        expect(ejectButton(wrapper).attributes('disabled')).toBeDefined()
        expect(ejectButton(wrapper).attributes('title')).toBe('Wait for the transfers to Kindle Paperwhite 5 to finish before ejecting it')

        store.transferJobs[0].status = 'done'
        await flushPromises()
        mockInvoke.on('eject_device', () => undefined)
        await ejectButton(wrapper).trigger('click')
        await flushPromises()
        expect(mockInvoke.callsTo('eject_device')[0].args).toEqual({ mountPath: '/Volumes/Kindle' })
        expect(store.device).toBeNull()
        expect(wrapper.text()).toContain('No device')
    })
})

describe('AppSidebar navigation', () => {
    const navTexts = (wrapper: ReturnType<typeof mountSidebar>['wrapper']) =>
        wrapper.findAll('.sidebar_nav > *').map(n => n.text().replace(/\s+/g, ' ').trim())

    it('links to Library, Discover and Settings, with Activity in between (no Device view)', () => {
        const { wrapper } = mountSidebar([])
        const links = wrapper.findAllComponents(RouterLinkStub).map(l => l.props('to'))
        expect(links).toEqual(['/library', '/discover', '/settings'])
        expect(navTexts(wrapper)).toEqual(['Library', 'Activity', 'Discover', 'Settings'])
        expect(wrapper.find('nav').attributes('aria-label')).toBe('Main')
    })

    it('links the connected e-reader to its books in the library view', () => {
        const { wrapper } = mountSidebar([paperwhite('a', '/Volumes/Kindle')])
        const link = wrapper.findAllComponents(RouterLinkStub).find(l => l.classes().includes('device-name'))!
        expect(link.props('to')).toEqual({ path: '/library', query: { scope: 'device' } })
        expect(link.attributes('title')).toBe('Show the books on Kindle')
        expect(link.find('.device-brand').text()).toBe('Kindle')
    })

    it('without an e-reader, says how to connect one', () => {
        const { wrapper } = mountSidebar([])
        const name = wrapper.find('.device-name')
        expect(name.element.tagName).toBe('DIV')
        expect(name.text()).toBe('No device')
        expect(name.attributes('title')).toBe('Connect a Kindle or Kobo with a USB cable: its books show up in your library.')
    })

    it('shows the user in their own colour, and names the switch', () => {
        const { wrapper } = mountSidebar([])
        const chip = wrapper.find('.user-chip')
        expect(chip.find('.user-chip_avatar').attributes('style')).toMatch(/background: var\(--cover-\d\)/)
    })

    it('names the language buttons and says which is on', () => {
        currentLang.value = 'en'
        const { wrapper } = mountSidebar([])
        const buttons = wrapper.findAll('.lang-toggle button')
        expect(buttons.map(b => [b.attributes('aria-label'), b.attributes('aria-pressed')])).toEqual([['Français', 'false'], ['English', 'true']])
    })
})

describe('AppSidebar MTP Kindles', () => {
    /** A 2024 Kindle plugged in, as `get_mtp_status` reports it. */
    const plugged = (state: MtpDeviceState, message?: string): MtpDevice => ({
        mountPath: 'mtp://usb-20-1.2', name: 'Kindle', state, message,
    })

    /** The sidebar with nothing connected but these MTP Kindles plugged in. */
    async function mountWithMtp(...mtp: MtpDevice[]) {
        const mounted = mountSidebar([])
        mounted.store.mtpDevices = mtp
        await flushPromises()
        return mounted
    }

    afterEach(() => { currentLang.value = 'en' })

    it('asks to turn MTP support on, with a link to the setting', async () => {
        const { wrapper } = await mountWithMtp(plugged('mtpDisabled', 'MTP support is off: enable it in Settings.'))
        const notice = wrapper.find('.device-notice')
        expect(notice.find('.device-notice-text').text())
            .toBe('A Kindle using MTP is connected — enable experimental MTP support in Settings')
        expect(notice.find('.device-notice-detail').exists()).toBe(false)
        const link = notice.findComponent(RouterLinkStub)
        expect(link.props('to')).toEqual({ path: '/settings', hash: '#mtp-setting' })
        expect(link.text()).toBe('Open Settings')
        // Named in place of "No device".
        expect(wrapper.find('.device-brand').text()).toBe('Kindle')
        expect(wrapper.text()).not.toContain('No device')
    })

    it.each([
        ['claimFailed', 'Librarium can’t open Kindle'],
        ['busy', 'Kindle is busy'],
        ['error', 'Can’t connect to Kindle'],
    ] as const)('explains %s by its state, with the backend’s message as the detail', async (state, text) => {
        const { wrapper } = await mountWithMtp(plugged(state, 'Quit Image Capture, then unplug and replug the Kindle.'))
        expect(wrapper.find('.device-notice-text').text()).toBe(text)
        expect(wrapper.find('.device-notice-detail').text()).toBe('Quit Image Capture, then unplug and replug the Kindle.')
        expect(wrapper.find('.device-notice-link').exists()).toBe(false)
    })

    it('translates by state, the backend’s message staying as it is', async () => {
        currentLang.value = 'fr'
        const { wrapper } = await mountWithMtp(plugged('claimFailed', 'Another app is using the Kindle.'))
        expect(wrapper.find('.device-notice-text').text()).toBe('Librarium ne peut pas ouvrir Kindle')
        expect(wrapper.find('.device-notice-detail').text()).toBe('Another app is using the Kindle.')
    })

    it('spins while connecting, and says an ejected Kindle is safe to unplug', async () => {
        const connecting = await mountWithMtp(plugged('connecting'))
        expect(connecting.wrapper.find('.device-notice-text').text()).toBe('Connecting to Kindle…')
        expect(connecting.wrapper.find('.device-notice [type="loader"]').classes()).toContain('spinning')

        const ejected = await mountWithMtp(plugged('ejected', 'Ejected: safe to unplug.'))
        expect(ejected.wrapper.find('.device-notice-text').text()).toBe('Kindle is safe to unplug')
        expect(ejected.wrapper.find('.device-notice-detail').exists()).toBe(false)
        expect(ejected.wrapper.find('.device-notice [type="loader"]').exists()).toBe(false)
    })

    it('shows connected MTP Kindles as devices, told apart by port, never by their mtp:// path', async () => {
        const { wrapper, store } = mountSidebar([paperwhite('a', 'mtp://usb-20-1.2'), paperwhite('b', 'mtp://usb-20-3')])
        store.mtpDevices = [
            { mountPath: 'mtp://usb-20-1.2', name: 'Kindle Paperwhite 5', state: 'ready', deviceId: 'a' },
            { mountPath: 'mtp://usb-20-3', name: 'Kindle Paperwhite 5', state: 'ready', deviceId: 'b' },
        ]
        await flushPromises()
        expect(wrapper.find('.device-notice').exists()).toBe(false)
        expect(wrapper.findAll('select.device-select option').map(o => o.text()))
            .toEqual(['Kindle Paperwhite 5 (MTP usb-20-1.2)', 'Kindle Paperwhite 5 (MTP usb-20-3)'])
        expect(wrapper.html()).not.toContain('mtp://')
    })

    it('lists the MTP Kindles that can’t be used under the connected device', async () => {
        const { wrapper, store } = mountSidebar([paperwhite('a', '/Volumes/Kindle')])
        store.mtpDevices = [plugged('busy', 'The Kindle keeps answering “busy”.')]
        await flushPromises()
        expect(wrapper.find('.device-brand').text()).toBe('Kindle')
        expect(wrapper.find('.device-model').text()).toBe('Paperwhite 5')
        expect(wrapper.find('.device-notice-text').text()).toBe('Kindle is busy')
    })
})
