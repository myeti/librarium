import type { TextKey } from '@/i18n'

/**
 * Every app command that has a keyboard shortcut or a native menu item, in
 * one place: the single list of the app's shortcuts.
 *
 * The keys live here; the actions live in the components (or App-level
 * code) that register a handler for an id with the shortcuts store
 * (`shortcutsStore.register`). The same handlers run whether the command
 * comes from the keyboard or from the menu bar: the native menu
 * (src-tauri/src/menu.rs, macOS) sends the command id, and the shortcuts
 * store runs it like the key (`runCommand`). **The accelerators shown in
 * the menu mirror the `keys` below**: `menu.test.ts` reads menu.rs and
 * fails when they differ. The Keyboard shortcuts dialog lists this table.
 *
 * Key combos are written `Mod+Shift+K`:
 * - `Mod` is ⌘ on macOS and Ctrl elsewhere (the platform's command key);
 *   `Ctrl`, `Alt`, `Shift` and `Meta` mean exactly that key;
 * - the last part is a `KeyboardEvent.key` value: a letter (case doesn't
 *   matter), or a name such as `Escape`, `Delete`, `ArrowLeft`;
 * - modifiers must match exactly: `Mod+A` doesn't fire on `Mod+Shift+A`.
 * A command with no `keys` is only in the menu bar (and wherever a button
 * runs it).
 *
 * Keys pressed inside a dialog never reach global shortcuts (the dialog on
 * top owns the keyboard), and menu commands don't run behind a dialog
 * either; keys typed in a text field don't fire a shortcut unless it says
 * `whileTyping`.
 *
 * Not listed because they aren't global: keys handled by a focused widget
 * (arrow keys in a menu, in the books or in the reader, Escape in a dialog,
 * Enter in a search field). The Keyboard shortcuts dialog describes the
 * books' keys separately.
 */
export interface ShortcutDefinition {
    /** Any of these combos triggers the command (the first is the one shown). Empty: menu only. */
    keys: string[]
    /** What it does, for the shortcut list. */
    description: TextKey
    /** Where the Keyboard shortcuts dialog lists it. */
    group: ShortcutGroup
    /** Also fires while a text field has focus (Mod-combos only, usually). */
    whileTyping?: boolean
    /** Also fires while a dialog is open (debug tools; commands the library lets run over its details). */
    whileModal?: boolean
}

export type ShortcutGroup = 'library' | 'view' | 'app'
export const SHORTCUT_GROUPS: readonly ShortcutGroup[] = ['library', 'view', 'app']

export const SHORTCUTS = {
    'library.import':         { keys: ['Mod+O'], description: 'shortcuts.import', group: 'library', whileTyping: true },
    // Over the details too (LibraryView lets them run when the details are the only dialog).
    'library.export':         { keys: [], description: 'shortcuts.export', group: 'library', whileModal: true },
    'library.detectDevice':   { keys: ['Mod+K'], description: 'shortcuts.detectDevice', group: 'library', whileTyping: true },
    'library.findDuplicates': { keys: [], description: 'shortcuts.findDuplicates', group: 'library' },
    'library.selectAll':      { keys: ['Mod+A'], description: 'shortcuts.selectAll', group: 'library' },
    'library.delete':         { keys: ['Delete', 'Backspace', 'Mod+Backspace'], description: 'shortcuts.delete', group: 'library' },
    'library.previous':       { keys: ['ArrowLeft'], description: 'shortcuts.previous', group: 'library' },
    'library.next':           { keys: ['ArrowRight'], description: 'shortcuts.next', group: 'library' },
    'library.close':          { keys: ['Escape'], description: 'shortcuts.close', group: 'library' },
    'library.search':         { keys: ['Mod+F'], description: 'shortcuts.search', group: 'library', whileTyping: true },
    'view.grid':              { keys: ['Mod+1'], description: 'shortcuts.gridView', group: 'view', whileTyping: true },
    'view.list':              { keys: ['Mod+2'], description: 'shortcuts.listView', group: 'view', whileTyping: true },
    'view.toggleDetails':     { keys: ['Mod+I'], description: 'shortcuts.toggleDetails', group: 'view', whileModal: true },
    'app.settings':           { keys: ['Mod+,'], description: 'shortcuts.settings', group: 'app', whileTyping: true },
    'app.about':              { keys: [], description: 'shortcuts.about', group: 'app' },
    'help.shortcuts':         { keys: ['Mod+/'], description: 'shortcuts.help', group: 'app', whileTyping: true },
    'help.logs':              { keys: [], description: 'shortcuts.logs', group: 'app' },
    'help.privacy':           { keys: [], description: 'shortcuts.privacy', group: 'app' },
    'debug.toggle':           { keys: ['Ctrl+Shift+D'], description: 'shortcuts.debug', group: 'app', whileTyping: true, whileModal: true },
} satisfies Record<string, ShortcutDefinition>

export type ShortcutId = keyof typeof SHORTCUTS

export const SHORTCUT_IDS = Object.keys(SHORTCUTS) as ShortcutId[]

/** Whether `id` is a known command (a menu event names one by its id). */
export function isShortcutId(id: unknown): id is ShortcutId {
    return typeof id === 'string' && Object.prototype.hasOwnProperty.call(SHORTCUTS, id)
}

/** A parsed key combo. */
export interface KeyCombo {
    key: string
    mod: boolean
    ctrl: boolean
    alt: boolean
    shift: boolean
    meta: boolean
}

/** Parses `Mod+Shift+K` (see SHORTCUTS for the syntax). Throws on an unknown modifier. */
export function parseCombo(combo: string): KeyCombo {
    const parts = combo.split('+')
    const key = parts.pop()
    if (!key) { throw new Error(`empty key combo "${combo}"`) }
    const result: KeyCombo = { key, mod: false, ctrl: false, alt: false, shift: false, meta: false }
    for (const part of parts) {
        const name = part.toLowerCase()
        if (name === 'mod' || name === 'ctrl' || name === 'alt' || name === 'shift' || name === 'meta') {
            result[name] = true
        } else {
            throw new Error(`unknown modifier "${part}" in "${combo}"`)
        }
    }
    return result
}

/** Whether `e` is exactly `combo` (`isMac` decides what `Mod` means). */
export function matchesCombo(e: KeyboardEvent, combo: KeyCombo, isMac: boolean): boolean {
    const wantCtrl = combo.ctrl || (combo.mod && !isMac)
    const wantMeta = combo.meta || (combo.mod && isMac)
    if (e.ctrlKey !== wantCtrl || e.metaKey !== wantMeta || e.altKey !== combo.alt || e.shiftKey !== combo.shift) {
        return false
    }
    // Letters arrive as 'a' or 'A' depending on Shift and Caps Lock.
    return combo.key.length === 1 ? e.key.toLowerCase() === combo.key.toLowerCase() : e.key === combo.key
}

const MAC_SYMBOLS: Record<string, string> = { mod: '⌘', meta: '⌘', ctrl: '⌃', alt: '⌥', shift: '⇧' }
const KEY_NAMES: Record<string, string> = {
    ArrowLeft: '←', ArrowRight: '→', ArrowUp: '↑', ArrowDown: '↓', Escape: 'Esc', Backspace: '⌫', Delete: 'Del',
}

/** The Tauri menu accelerator of a combo (`Mod+O` → `CmdOrCtrl+O`), as menu.rs writes it. */
export function toMenuAccelerator(combo: string): string {
    const c = parseCombo(combo)
    const parts = [
        c.mod && 'CmdOrCtrl', c.ctrl && 'Ctrl', c.alt && 'Alt', c.shift && 'Shift', c.meta && 'Super',
    ].filter((p): p is string => Boolean(p))
    return [...parts, c.key.length === 1 ? c.key.toUpperCase() : c.key].join('+')
}

/** How to show a combo to the user: `⌘⇧K` on macOS, `Ctrl+Shift+K` elsewhere. */
export function formatCombo(combo: string, isMac: boolean): string {
    const c = parseCombo(combo)
    const key = KEY_NAMES[c.key] ?? (c.key.length === 1 ? c.key.toUpperCase() : c.key)
    if (isMac) {
        // macOS order: ⌃ ⌥ ⇧ ⌘
        const mods = [c.ctrl && 'ctrl', c.alt && 'alt', c.shift && 'shift', (c.mod || c.meta) && 'mod']
            .filter((m): m is string => Boolean(m)).map(m => MAC_SYMBOLS[m])
        return mods.join('') + key
    }
    const mods = [(c.ctrl || c.mod) && 'Ctrl', c.alt && 'Alt', c.shift && 'Shift', c.meta && 'Win']
        .filter((m): m is string => Boolean(m))
    return [...mods, key].join('+')
}

/** Whether this is macOS (where `Mod` is ⌘). */
export function isMacPlatform(): boolean {
    return typeof navigator !== 'undefined' && /Mac|iPhone|iPad/.test(navigator.userAgent)
}
