import { afterEach, describe, expect, it, vi } from 'vitest'

vi.mock('@tauri-apps/api/core', () => import('@/test/tauri'))
import { cleanup, mountUi, testPinia } from '@/test/mountUi'
import ShortcutsDialog from './ShortcutsDialog.vue'
import { useShortcutsStore } from '@/stores/shortcuts'
import { SHORTCUTS } from '@/constants/shortcuts'
import en from '@/i18n/en'

/** The dialog's rows: description → keys as shown. */
function rows(): Record<string, string> {
    const result: Record<string, string> = {}
    for (const row of document.querySelectorAll('.shortcuts__row')) {
        const keys = [...row.querySelectorAll('kbd')].map(k => k.textContent.trim()).join(' ')
        result[row.querySelector('dt')!.textContent.trim()] = keys
    }
    return result
}

function open(isMac: boolean) {
    testPinia()
    useShortcutsStore().isMac = isMac
    return mountUi(ShortcutsDialog)
}

afterEach(() => cleanup())

describe('ShortcutsDialog', () => {
    it('lists every command that has keys, from constants/shortcuts.ts', () => {
        open(true)
        const shown = rows()
        for (const def of Object.values(SHORTCUTS)) {
            const [section, key] = def.description.split('.')
            const text = (en as unknown as Record<string, Record<string, string>>)[section][key]
            if (def.keys.length > 0) {
                expect(shown[text], text).toBeDefined()
            } else {
                // Menu-only commands have no keys to show.
                expect(shown[text], text).toBeUndefined()
            }
        }
    })

    it('writes the keys the way the platform does', () => {
        open(true)
        expect(rows()['Import books']).toBe('⌘O')
        expect(rows()['Open Settings']).toBe('⌘,')
        expect(rows()['Delete the selected books']).toBe('Del ⌫ ⌘⌫')
        cleanup()
        open(false)
        expect(rows()['Import books']).toBe('Ctrl+O')
        expect(rows()['Show or hide the debug console']).toBe('Ctrl+Shift+D')
    })

    it('groups them, and explains the keys of the books', () => {
        open(true)
        const headings = [...document.querySelectorAll('.shortcuts__heading')].map(h => h.textContent.trim())
        expect(headings).toEqual(['Library', 'View', 'App', 'In the books'])
        expect(rows()['Select or unselect the book (Shift: a range)']).toBe('Space')
        expect(document.body.textContent).toContain('the same commands are in the menu bar')
    })
})
