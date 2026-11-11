import { afterEach, describe, expect, it, vi } from 'vitest'
import { mount, type VueWrapper } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { nextTick } from 'vue'

vi.mock('@tauri-apps/api/core', () => import('@/test/tauri'))
import RestoreBackupDialog from './RestoreBackupDialog.vue'
import { i18nPlugin } from '@/i18n'
import { useBackupStore } from '@/stores/backup'
import { useImportsStore } from '@/stores/imports'
import { useUsersStore } from '@/stores/users'
import type { BackupManifest } from '@/types'

const MANIFEST: BackupManifest = {
    app: 'Librarium', format: 1, appVersion: '0.6.0', createdAt: '2026-11-10T13:32:00Z',
    users: ['Clara'], files: 12, bytes: 5 * 1024 * 1024,
}

describe('RestoreBackupDialog', () => {
    let wrapper: VueWrapper | null = null

    afterEach(() => {
        wrapper?.unmount()
        wrapper = null
        document.body.innerHTML = ''
    })

    function open() {
        const pinia = createPinia()
        setActivePinia(pinia)
        const users = useUsersStore()
        users.libraryRoot = '/Users/me/Documents/Librarium'
        users.users = ['Alice', 'Zoé']
        wrapper = mount(RestoreBackupDialog, {
            props: { path: '/Backups/b.zip', manifest: MANIFEST },
            global: { plugins: [pinia, i18nPlugin], stubs: { 'vue-feather': true } },
            attachTo: document.body,
        })
        const dialog = document.body.querySelector('[role="alertdialog"]') as HTMLElement
        const buttons = () => Array.from(dialog.querySelectorAll('button'))
        const restoreButton = () => buttons().find(b => b.textContent?.includes('Erase and restore'))!
        return { dialog, restoreButton, checkbox: dialog.querySelector('input[type="checkbox"]') as HTMLInputElement }
    }

    it('says what will be erased and by what, focus on Cancel', () => {
        const { dialog } = open()
        const text = dialog.textContent ?? ''
        expect(text).toContain('Alice, Zoé')
        expect(text).toContain('Clara')
        expect(text).toContain('0.6.0')
        expect(text).toContain('/Users/me/Documents/Librarium')
        expect(document.activeElement?.textContent).toContain('Cancel')
    })

    it('restores only once "I understand" is ticked', async () => {
        const { restoreButton, checkbox } = open()
        const restore = vi.spyOn(useBackupStore(), 'restoreFrom').mockResolvedValue({ users: ['Clara'], files: 12, cancelled: false })
        expect(restoreButton().disabled).toBe(true)
        restoreButton().click()
        expect(restore).not.toHaveBeenCalled()

        checkbox.click()
        await nextTick()
        expect(restoreButton().disabled).toBe(false)
        restoreButton().click()
        await nextTick()
        expect(restore).toHaveBeenCalledWith('/Backups/b.zip', MANIFEST)
    })

    it('waits while books are being imported', async () => {
        const { dialog, restoreButton, checkbox } = open()
        useImportsStore().pending = 1
        checkbox.click()
        await nextTick()
        expect(dialog.textContent).toContain('wait until it’s done')
        expect(restoreButton().disabled).toBe(true)
    })

    it('shows the progress, and stops being dismissible, while restoring', async () => {
        const { dialog } = open()
        const backup = useBackupStore()
        backup.running = 'restore'
        backup.progress = { phase: 'copying', filesDone: 6, filesTotal: 12, bytesDone: 50, bytesTotal: 100 }
        await nextTick()
        expect(dialog.querySelector('[role="progressbar"]')?.getAttribute('aria-valuenow')).toBe('50')
        expect(dialog.textContent).toContain('6 of 12 files')
        dialog.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }))
        await nextTick()
        expect(wrapper!.emitted('close')).toBeUndefined()

        backup.progress = { ...backup.progress, phase: 'replacing' }
        await nextTick()
        const cancel = Array.from(dialog.querySelectorAll('button')).find(b => b.textContent?.includes('Cancel'))!
        expect(cancel.disabled).toBe(true)
    })
})
