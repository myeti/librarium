import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import { DEFER_MS, MENU_ECHO_MS, useShortcutsStore } from './shortcuts'
import { useModalsStore } from './modals'
import { formatCombo, matchesCombo, parseCombo, SHORTCUTS, SHORTCUT_IDS } from '@/constants/shortcuts'
import en from '@/i18n/en'

function press(key: string, init: KeyboardEventInit = {}, target: EventTarget = document.body): KeyboardEvent {
    const e = new KeyboardEvent('keydown', { key, bubbles: true, cancelable: true, ...init })
    target.dispatchEvent(e)
    return e
}

describe('shortcuts store', () => {
    beforeEach(() => {
        setActivePinia(createPinia())
        const store = useShortcutsStore()
        store.isMac = true
        store.install(document)
    })

    afterEach(() => {
        useShortcutsStore().reset()
        document.body.innerHTML = ''
    })

    it('runs the handler registered for a shortcut, and prevents the default', () => {
        const store = useShortcutsStore()
        const importFiles = vi.fn()
        store.register('library.import', importFiles)
        const e = press('o', { metaKey: true })
        expect(importFiles).toHaveBeenCalledOnce()
        expect(e.defaultPrevented).toBe(true)
        expect(store.active).toEqual(['library.import'])
    })

    it('Mod is ⌘ on macOS and Ctrl elsewhere; other modifiers must match exactly', () => {
        const store = useShortcutsStore()
        const handler = vi.fn()
        store.register('library.import', handler)
        press('o', { ctrlKey: true })                 // Ctrl on a Mac: not Mod
        press('o', { metaKey: true, shiftKey: true }) // extra Shift
        expect(handler).not.toHaveBeenCalled()
        store.isMac = false
        press('o', { ctrlKey: true })
        expect(handler).toHaveBeenCalledOnce()
    })

    it('one global listener: installing twice does not double the calls', () => {
        const store = useShortcutsStore()
        store.install(document)
        const handler = vi.fn()
        store.register('library.import', handler)
        press('o', { metaKey: true })
        expect(handler).toHaveBeenCalledOnce()
    })

    it('ignores keys while a dialog is open, except shortcuts meant for that', () => {
        const store = useShortcutsStore()
        const selectAll = vi.fn()
        const debug = vi.fn()
        store.register('library.selectAll', selectAll)
        store.register('debug.toggle', debug)
        useModalsStore().open('ConfirmDialog')
        const e = press('a', { metaKey: true })
        expect(selectAll).not.toHaveBeenCalled()
        expect(e.defaultPrevented).toBe(false)
        press('D', { ctrlKey: true, shiftKey: true })
        expect(debug).toHaveBeenCalledOnce()
    })

    it('ignores keys typed in a text field, unless the shortcut allows it', () => {
        const store = useShortcutsStore()
        const selectAll = vi.fn()
        const importFiles = vi.fn()
        store.register('library.selectAll', selectAll)
        store.register('library.import', importFiles)
        const input = document.createElement('input')
        document.body.appendChild(input)
        input.focus()
        press('a', { metaKey: true }, input)          // select the field's text, not the books
        expect(selectAll).not.toHaveBeenCalled()
        press('o', { metaKey: true }, input)          // ⌘O still imports
        expect(importFiles).toHaveBeenCalledOnce()
    })

    it('a checkbox is not a text field', () => {
        const store = useShortcutsStore()
        const del = vi.fn()
        store.register('library.delete', del)
        const box = document.createElement('input')
        box.type = 'checkbox'
        document.body.appendChild(box)
        box.focus()
        press('Delete', {}, box)
        expect(del).toHaveBeenCalledOnce()
    })

    it('ignores keys a focused widget already handled (preventDefault)', () => {
        const store = useShortcutsStore()
        const close = vi.fn()
        store.register('library.close', close)
        const menu = document.createElement('div')
        document.body.appendChild(menu)
        menu.addEventListener('keydown', e => e.preventDefault())
        press('Escape', {}, menu)
        expect(close).not.toHaveBeenCalled()
    })

    it('the newest handler wins; returning false passes the key to the previous one', () => {
        const store = useShortcutsStore()
        const panel = vi.fn(() => false as const)       // nothing to close
        const view = vi.fn()
        store.register('library.close', view)
        store.register('library.close', panel)
        const e = press('Escape')
        expect(panel).toHaveBeenCalledOnce()
        expect(view).toHaveBeenCalledOnce()
        expect(e.defaultPrevented).toBe(true)
    })

    it('a handler that returns false everywhere leaves the key alone', () => {
        const store = useShortcutsStore()
        store.register('library.delete', () => false)
        expect(press('Backspace').defaultPrevented).toBe(false)
    })

    it('unregisters (with the function returned by register, or by reference)', () => {
        const store = useShortcutsStore()
        const a = vi.fn()
        const b = vi.fn()
        const off = store.register('library.next', a)
        store.register('library.previous', b)
        off()
        store.unregister('library.previous', b)
        press('ArrowRight')
        press('ArrowLeft')
        expect(a).not.toHaveBeenCalled()
        expect(b).not.toHaveBeenCalled()
        expect(store.active).toEqual([])
    })

    it('stops listening once uninstalled', () => {
        const store = useShortcutsStore()
        const handler = vi.fn()
        store.register('library.import', handler)
        store.uninstall()
        press('o', { metaKey: true })
        expect(handler).not.toHaveBeenCalled()
    })
})

describe('commands from the menu bar and buttons', () => {
    beforeEach(() => {
        setActivePinia(createPinia())
        const store = useShortcutsStore()
        store.isMac = true
        store.install(document)
    })

    afterEach(() => {
        useShortcutsStore().reset()
        vi.useRealTimers()
    })

    it('runs the same handler as the key, without an event', () => {
        const store = useShortcutsStore()
        const handler = vi.fn()
        store.register('library.import', handler)
        expect(store.runCommand('library.import', 'menu')).toBe(true)
        expect(handler).toHaveBeenCalledWith(undefined)
    })

    it('ignores unknown ids and commands nobody handles', () => {
        const store = useShortcutsStore()
        expect(store.runCommand('rm -rf', 'menu')).toBe(false)
        expect(store.runCommand('library.export', 'menu')).toBe(false)
    })

    it('passes on to older handlers when the newest returns false', () => {
        const store = useShortcutsStore()
        const fallback = vi.fn()
        store.register('library.search', fallback)
        store.register('library.search', () => false)
        expect(store.runCommand('library.search')).toBe(true)
        expect(fallback).toHaveBeenCalledTimes(1)
    })

    it('does nothing behind a dialog, unless the command works in dialogs', () => {
        const store = useShortcutsStore()
        const settings = vi.fn()
        const debug = vi.fn()
        store.register('app.settings', settings)
        store.register('debug.toggle', debug)
        useModalsStore().open('ConfirmDialog')
        expect(store.runCommand('app.settings', 'menu')).toBe(false)
        expect(settings).not.toHaveBeenCalled()
        expect(store.runCommand('debug.toggle', 'menu')).toBe(true)
    })

    it('drops the menu’s echo of a key press it already handled', () => {
        vi.useFakeTimers()
        const store = useShortcutsStore()
        const handler = vi.fn()
        store.register('library.import', handler)
        press('o', { metaKey: true })
        expect(store.runCommand('library.import', 'menu')).toBe(true)
        expect(handler).toHaveBeenCalledTimes(1)
        // A real click later runs again; so does a button right away.
        vi.advanceTimersByTime(MENU_ECHO_MS + 1)
        store.runCommand('library.import', 'menu')
        store.runCommand('library.import', 'button')
        expect(handler).toHaveBeenCalledTimes(3)
    })

    it('runs a deferred command once its screen registers, if it is still wanted', () => {
        vi.useFakeTimers()
        const store = useShortcutsStore()
        const focusSearch = vi.fn()
        store.runWhenRegistered('library.search')
        store.register('library.search', focusSearch)
        expect(focusSearch).toHaveBeenCalledTimes(1)
        // Only once.
        store.register('library.search', focusSearch)
        expect(focusSearch).toHaveBeenCalledTimes(1)

        const late = vi.fn()
        store.runWhenRegistered('view.toggleDetails')
        vi.advanceTimersByTime(DEFER_MS + 1)
        store.register('view.toggleDetails', late)
        expect(late).not.toHaveBeenCalled()
    })
})

describe('shortcut list (constants/shortcuts.ts)', () => {
    it('every command has keys that parse (or none: menu only) and a description in the translations', () => {
        for (const id of SHORTCUT_IDS) {
            const def = SHORTCUTS[id]
            def.keys.forEach(k => expect(() => parseCombo(k), k).not.toThrow())
            const [section, key] = def.description.split('.')
            expect((en as unknown as Record<string, Record<string, string>>)[section]?.[key], id).toBeTypeOf('string')
        }
    })

    it('no two shortcuts share a key combo', () => {
        const seen = new Map<string, string>()
        for (const id of SHORTCUT_IDS) {
            for (const combo of SHORTCUTS[id].keys) {
                const c = parseCombo(combo)
                const norm = JSON.stringify({ ...c, key: c.key.toLowerCase() })
                expect(seen.get(norm), `${combo} is used by ${seen.get(norm)} and ${id}`).toBeUndefined()
                seen.set(norm, id)
            }
        }
    })

    it('parses and matches combos', () => {
        const combo = parseCombo('Mod+Shift+K')
        expect(combo).toMatchObject({ key: 'K', mod: true, shift: true, ctrl: false })
        const e = new KeyboardEvent('keydown', { key: 'k', metaKey: true, shiftKey: true })
        expect(matchesCombo(e, combo, true)).toBe(true)
        expect(matchesCombo(e, combo, false)).toBe(false)
        expect(() => parseCombo('Hyper+K')).toThrow(/unknown modifier/)
    })

    it('formats combos for each platform', () => {
        expect(formatCombo('Mod+Shift+K', true)).toBe('⇧⌘K')
        expect(formatCombo('Mod+Shift+K', false)).toBe('Ctrl+Shift+K')
        expect(formatCombo('Ctrl+Shift+D', true)).toBe('⌃⇧D')
        expect(formatCombo('ArrowLeft', false)).toBe('←')
    })
})
