import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import { describe, expect, it } from 'vitest'
import { SHORTCUTS, isShortcutId, toMenuAccelerator } from './shortcuts'

/**
 * The native menu bar (src-tauri/src/menu.rs) shows accelerators for the
 * app's commands; the keys themselves are handled by the shortcuts store
 * from src/constants/shortcuts.ts. Both must say the same thing: this test
 * reads menu.rs and compares.
 */
// Vitest runs from the project root.
const menuSource = readFileSync(join(process.cwd(), 'src-tauri/src/menu.rs'), 'utf8')
const ENTRY = /custom\("([\w.]+)",\s*"[^"]*",\s*"[^"]*",\s*(None|Some\("([^"]+)"\))\)/g
const entries = [...menuSource.matchAll(ENTRY)].map(m => ({ id: m[1], accelerator: m[3] ?? null }))

describe('native menu ↔ shortcuts', () => {
    it('finds the menu’s commands', () => {
        expect(entries.length).toBeGreaterThanOrEqual(10)
    })

    it('only sends commands the app knows', () => {
        expect(entries.filter(e => !isShortcutId(e.id)).map(e => e.id)).toEqual([])
    })

    it('shows the same accelerator as the first key of each command', () => {
        for (const { id, accelerator } of entries) {
            if (!isShortcutId(id)) { continue }
            const keys: string[] = SHORTCUTS[id].keys
            expect({ id, accelerator }).toEqual({ id, accelerator: keys.length > 0 ? toMenuAccelerator(keys[0]) : null })
        }
    })

    it('has a menu item for every command that has no key (or it could never run)', () => {
        const inMenu = new Set(entries.map(e => e.id))
        const keyless = Object.entries(SHORTCUTS).filter(([, def]) => def.keys.length === 0).map(([id]) => id)
        expect(keyless.filter(id => !inMenu.has(id))).toEqual([])
    })
})

describe('toMenuAccelerator', () => {
    it('writes combos the way Tauri menus do', () => {
        expect(toMenuAccelerator('Mod+O')).toBe('CmdOrCtrl+O')
        expect(toMenuAccelerator('Mod+,')).toBe('CmdOrCtrl+,')
        expect(toMenuAccelerator('Mod+Shift+k')).toBe('CmdOrCtrl+Shift+K')
        expect(toMenuAccelerator('Ctrl+Shift+D')).toBe('Ctrl+Shift+D')
    })
})
