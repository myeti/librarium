/**
 * The keyboard and selection model of the library collection (grid or
 * list), as pure functions so it can be tested without a DOM.
 *
 * The collection is one focusable widget (a listbox with a roving tabindex:
 * Tab reaches the active book, the arrows move between books). Keys:
 *
 * | Key | Does |
 * |-----|------|
 * | ← → | previous / next book |
 * | ↑ ↓ | the book above / below (one row = `columns` books) |
 * | Home / End | first / last book |
 * | PageUp / PageDown | one screenful of rows up / down |
 * | Shift + any of those | extends the selection from the anchor to the new book |
 * | Space | selects / deselects the active book (Shift+Space: range) |
 * | Enter | opens the book in the detail panel |
 * | ContextMenu, Shift+F10 | opens the book's menu |
 *
 * Mouse: a click opens the book; Cmd-click (Ctrl-click elsewhere) toggles
 * it in the selection; Shift-click selects the range from the anchor.
 *
 * Global shortcuts (⌘A, Delete, Escape) are not handled here: they go
 * through the shortcuts store (constants/shortcuts.ts).
 */

export type NavKey = 'ArrowLeft' | 'ArrowRight' | 'ArrowUp' | 'ArrowDown' | 'Home' | 'End' | 'PageUp' | 'PageDown'

const NAV_KEYS: readonly string[] = ['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown', 'Home', 'End', 'PageUp', 'PageDown']

export function isNavKey(key: string): key is NavKey {
    return NAV_KEYS.includes(key)
}

/** What a key press asks the collection to do. */
export type KeyIntent =
    | { type: 'move', key: NavKey, extend: boolean }
    | { type: 'toggle', range: boolean }
    | { type: 'open' }
    | { type: 'menu' }

/**
 * The collection's reading of a keydown, or null for keys it leaves alone
 * (letters, ⌘A, Delete, Escape…: those are global shortcuts, or nothing).
 * Navigation with Ctrl/⌘/Alt held is left alone too (browser and OS
 * shortcuts use those).
 */
export function keyIntent(e: Pick<KeyboardEvent, 'key' | 'shiftKey' | 'ctrlKey' | 'metaKey' | 'altKey'>): KeyIntent | null {
    const commandHeld = e.ctrlKey || e.metaKey || e.altKey
    if (e.key === 'ContextMenu' || (e.key === 'F10' && e.shiftKey && !commandHeld)) { return { type: 'menu' } }
    if (commandHeld) { return null }
    if (isNavKey(e.key)) { return { type: 'move', key: e.key, extend: e.shiftKey } }
    if (e.key === ' ' || e.key === 'Spacebar') { return { type: 'toggle', range: e.shiftKey } }
    if (e.key === 'Enter' && !e.shiftKey) { return { type: 'open' } }
    return null
}

/**
 * The index the focus moves to from `index` for `key`, in a collection of
 * `count` items laid out in rows of `columns` (`pageRows` rows per screen).
 * Moves stop at the edges (no wrapping from the first book to the last).
 * Returns -1 when the collection is empty.
 */
export function navigate(key: NavKey, index: number, count: number, columns: number, pageRows = 1): number {
    if (count <= 0) { return -1 }
    const cols = Math.max(1, columns)
    const last = count - 1
    if (index < 0 || index > last) {
        return key === 'End' || key === 'ArrowUp' || key === 'PageUp' || key === 'ArrowLeft' ? last : 0
    }
    const col = index % cols
    const lastRowStart = Math.floor(last / cols) * cols
    switch (key) {
    case 'ArrowLeft':  return Math.max(0, index - 1)
    case 'ArrowRight': return Math.min(last, index + 1)
    case 'ArrowUp':    return index - cols >= 0 ? index - cols : index
    case 'ArrowDown':
        if (index + cols <= last) { return index + cols }
        // From the row above a shorter last row: its last book.
        return index < lastRowStart ? last : index
    case 'Home':       return 0
    case 'End':        return last
    case 'PageUp': {
        const target = index - cols * Math.max(1, pageRows)
        return target >= 0 ? target : col
    }
    case 'PageDown': {
        const target = index + cols * Math.max(1, pageRows)
        if (target <= last) { return target }
        return Math.min(last, lastRowStart + col)
    }
    }
}

/** What a click on a book asks for. */
export type ClickIntent = 'open' | 'toggle' | 'range'

/** Shift-click: range; ⌘-click on macOS, Ctrl-click elsewhere: toggle; else open. */
export function clickIntent(e: Pick<MouseEvent, 'shiftKey' | 'ctrlKey' | 'metaKey'>, isMac: boolean): ClickIntent {
    if (e.shiftKey) { return 'range' }
    if (isMac ? e.metaKey : e.ctrlKey) { return 'toggle' }
    return 'open'
}

/** The ids from position `a` to position `b` of `ids`, inclusive, whichever comes first. */
export function idsBetween(ids: readonly string[], a: number, b: number): string[] {
    if (ids.length === 0) { return [] }
    const clamp = (i: number) => Math.min(Math.max(0, i), ids.length - 1)
    const from = Math.min(clamp(a), clamp(b))
    const to = Math.max(clamp(a), clamp(b))
    return ids.slice(from, to + 1)
}

function sameSet(a: ReadonlySet<string>, b: ReadonlySet<string>): boolean {
    if (a.size !== b.size) { return false }
    for (const id of a) {
        if (!b.has(id)) { return false }
    }
    return true
}

/**
 * Range selection, as in a file manager: Shift+click or Shift+arrow selects
 * every book from the *anchor* (the last book clicked, toggled or moved to
 * without Shift) to the target, on top of what was selected when the
 * anchor was set. Moving the target back shrinks the range again:
 *
 *     selection = base ∪ books(anchor … target)
 *
 * If the selection was changed by something else in between (Clear,
 * Escape, ⌘A, a filter), that new selection becomes the base.
 */
export class RangeSelection {
    private anchorId: string | null = null
    private base: Set<string> = new Set()
    /** The selection as this object last left it, to notice outside changes. */
    private lastResult: Set<string> | null = null

    /** The anchor, if it is still one of `ids`. */
    anchorIndex(ids: readonly string[]): number {
        return this.anchorId === null ? -1 : ids.indexOf(this.anchorId)
    }

    /** Starts the next range at `id`, on top of `selection`. */
    setAnchor(id: string | null, selection: ReadonlySet<string>): void {
        this.anchorId = id
        this.base = new Set(selection)
        this.lastResult = new Set(selection)
    }

    /**
     * The selection after extending to position `target` of `ids`.
     * Without a usable anchor, `fallbackAnchor` (the active book) becomes
     * the anchor.
     */
    extend(ids: readonly string[], target: number, selection: ReadonlySet<string>, fallbackAnchor: number): Set<string> {
        let anchor = this.anchorIndex(ids)
        if (anchor === -1) {
            anchor = fallbackAnchor >= 0 ? fallbackAnchor : target
            this.setAnchor(ids[anchor] ?? null, selection)
        } else if (this.lastResult && !sameSet(this.lastResult, selection)) {
            this.base = new Set(selection)
        }
        const result = new Set(this.base)
        for (const id of idsBetween(ids, anchor, target)) { result.add(id) }
        this.lastResult = new Set(result)
        return result
    }
}

/**
 * The active book after the list changed (filter, sort, delete): the same
 * book if it is still listed; else the one that took its place (same
 * position, or the last one); null when the list is empty.
 */
export function nextActiveId(previous: readonly string[], next: readonly string[], activeId: string | null): string | null {
    if (activeId === null) { return null }
    if (next.includes(activeId)) { return activeId }
    if (next.length === 0) { return null }
    const was = previous.indexOf(activeId)
    if (was === -1) { return null }
    return next[Math.min(was, next.length - 1)]
}
