import { defineStore } from 'pinia'
import type { App } from 'vue'
import {
    SHORTCUTS, SHORTCUT_IDS, isMacPlatform, isShortcutId, matchesCombo, parseCombo,
    type KeyCombo, type ShortcutDefinition, type ShortcutId,
} from '@/constants/shortcuts'
import { useModalsStore } from '@/stores/modals'
import { isTypingTarget } from '@/utils/dom'

/**
 * What a shortcut does. Return `false` when it didn't apply (nothing
 * selected, no panel to close…): the key then keeps its default behaviour
 * and the next handler registered for the same shortcut gets a chance.
 * `e` is the key event, or undefined when the command came from the menu
 * bar or a button.
 */
export type ShortcutHandler = (e?: KeyboardEvent) => boolean | void

/** Where a command came from, other than a key. */
export type CommandSource = 'menu' | 'button'

/**
 * A menu event for the command a key just ran is the same key press seen
 * twice (the webview and the native menu both reacting to an accelerator):
 * dropped when it arrives within this delay.
 */
export const MENU_ECHO_MS = 500

/** How long a command waits for its handler (`runWhenRegistered`), e.g. while the library view opens. */
export const DEFER_MS = 3000

interface ShortcutsState {
    /** Shortcuts that currently have at least one handler (for a help list). */
    active: ShortcutId[]
    /** Whether the global listener is installed. */
    installed: boolean
    /** `Mod` means ⌘ (macOS) or Ctrl. */
    isMac: boolean
}

// Handlers are functions owned by components: kept out of the reactive
// state on purpose. Newest last; the newest handler runs first.
const handlers = new Map<ShortcutId, ShortcutHandler[]>()
const combos = new Map<ShortcutId, KeyCombo[]>(
    SHORTCUT_IDS.map(id => [id, SHORTCUTS[id].keys.map(parseCombo)]),
)
let listener: ((e: KeyboardEvent) => void) | null = null
let listenerTarget: Window | Document | null = null
/** The last command a key ran, to drop its echo from the menu bar. */
let lastKeyCommand: { id: ShortcutId, at: number } | null = null
/** A command waiting for its handler to be registered (`runWhenRegistered`). */
let deferred: { id: ShortcutId, until: number } | null = null

/**
 * Global keyboard shortcuts: one keydown listener for the whole app.
 *
 * The keys are declared once in `src/constants/shortcuts.ts`; components
 * register what a shortcut does while they are on screen:
 *
 *     mounted()       { this.shortcutsStore.register('library.import', this.importFiles) }
 *     beforeUnmount() { this.shortcutsStore.unregister('library.import', this.importFiles) }
 *
 * The listener ignores a key when:
 * - another handler already called `preventDefault()` (a focused widget
 *   used it: a menu, a text field);
 * - a dialog is open (modals store), unless the shortcut is `whileModal`;
 * - a text field has focus, unless the shortcut is `whileTyping`.
 * A shortcut whose handler runs gets `preventDefault()` (so ⌘A doesn't
 * also select the page's text), unless the handler returns `false`.
 *
 * The same handlers run for the native menu bar and for buttons that do
 * what a shortcut does (`runCommand`), with the same rules (nothing runs
 * behind a dialog). A command whose screen isn't shown yet can wait for it
 * (`runWhenRegistered`): the menu's "Search" from Settings opens the library,
 * then focuses its search field.
 *
 * Installed once by `shortcutsPlugin` (main.ts).
 */
export const useShortcutsStore = defineStore('shortcuts', {
    state: (): ShortcutsState => ({
        active: [],
        installed: false,
        isMac: isMacPlatform(),
    }),

    actions: {
        /**
         * Makes `handler` run for shortcut `id` until unregistered. Returns
         * a function that unregisters it (same as `unregister(id, handler)`).
         */
        register(id: ShortcutId, handler: ShortcutHandler): () => void {
            const list = handlers.get(id) ?? []
            list.push(handler)
            handlers.set(id, list)
            this.refreshActive()
            // A command was waiting for this screen: run it now.
            if (deferred?.id === id) {
                const stillWanted = Date.now() <= deferred.until
                deferred = null
                if (stillWanted) { handler() }
            }
            return () => this.unregister(id, handler)
        },

        /** Removes one handler (the same function that was registered). */
        unregister(id: ShortcutId, handler: ShortcutHandler) {
            const list = handlers.get(id)
            if (!list) { return }
            const idx = list.lastIndexOf(handler)
            if (idx !== -1) { list.splice(idx, 1) }
            if (list.length === 0) { handlers.delete(id) }
            this.refreshActive()
        },

        refreshActive() {
            this.active = SHORTCUT_IDS.filter(id => (handlers.get(id)?.length ?? 0) > 0)
        },

        /**
         * Runs the shortcut matching `e`, if any. Returns whether one ran.
         * Called by the global listener; public for tests.
         */
        handleKeydown(e: KeyboardEvent): boolean {
            if (e.defaultPrevented || e.isComposing) { return false }
            const modalOpen = useModalsStore().hasOpenModal
            const typing = isTypingTarget(e.target) || isTypingTarget(document.activeElement)

            for (const id of SHORTCUT_IDS) {
                const def: ShortcutDefinition = SHORTCUTS[id]
                if (!combos.get(id)!.some(c => matchesCombo(e, c, this.isMac))) { continue }
                if (modalOpen && !def.whileModal) { continue }
                if (typing && !def.whileTyping) { continue }
                if (this.runHandlers(id, e)) {
                    e.preventDefault()
                    lastKeyCommand = { id, at: Date.now() }
                    return true
                }
            }
            return false
        },

        /** Runs `id`'s handlers, newest first, until one doesn't return `false`. */
        runHandlers(id: ShortcutId, e?: KeyboardEvent): boolean {
            const list = [...(handlers.get(id) ?? [])].reverse()
            return list.some(handler => handler(e) !== false)
        },

        /**
         * Runs command `id` as if its key had been pressed, for the menu bar
         * (an id from a menu event: unknown ids are ignored) or a button.
         * Like a key, it doesn't run while a dialog is open (unless the
         * command is `whileModal`). Returns whether a handler ran.
         */
        runCommand(id: string, source: CommandSource = 'button'): boolean {
            if (!isShortcutId(id)) { return false }
            if (useModalsStore().hasOpenModal && !(SHORTCUTS[id] as ShortcutDefinition).whileModal) { return false }
            if (source === 'menu' && lastKeyCommand?.id === id && Date.now() - lastKeyCommand.at < MENU_ECHO_MS) {
                // The key press already ran it (see MENU_ECHO_MS).
                return true
            }
            return this.runHandlers(id)
        },

        /**
         * Runs command `id` as soon as a handler is registered for it (within
         * DEFER_MS): for a command whose screen is opening. Only the latest
         * request waits.
         */
        runWhenRegistered(id: ShortcutId) {
            deferred = { id, until: Date.now() + DEFER_MS }
        },

        /** Installs the single global keydown listener (idempotent). */
        install(target: Window | Document = window) {
            if (listener) { return }
            listener = (e: KeyboardEvent) => { this.handleKeydown(e) }
            listenerTarget = target
            target.addEventListener('keydown', listener as EventListener)
            this.installed = true
        },

        uninstall() {
            if (listener && listenerTarget) {
                listenerTarget.removeEventListener('keydown', listener as EventListener)
            }
            listener = null
            listenerTarget = null
            this.installed = false
        },

        /** Drops every handler (tests). */
        reset() {
            handlers.clear()
            lastKeyCommand = null
            deferred = null
            this.uninstall()
            this.refreshActive()
        },
    },
})

/**
 * Vue plugin: installs the global shortcut listener. Use after Pinia:
 * `app.use(createPinia()); app.use(shortcutsPlugin)`.
 */
export const shortcutsPlugin = {
    install(_app: App) {
        useShortcutsStore().install(window)
    },
}
