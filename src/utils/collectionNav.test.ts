import { describe, expect, it } from 'vitest'
import { clickIntent, idsBetween, keyIntent, navigate, nextActiveId, RangeSelection } from './collectionNav'

function key(k: string, mods: Partial<Record<'shiftKey' | 'ctrlKey' | 'metaKey' | 'altKey', boolean>> = {}) {
    return { key: k, shiftKey: false, ctrlKey: false, metaKey: false, altKey: false, ...mods }
}

describe('keyIntent', () => {
    it('reads arrows, Home/End and PageUp/PageDown as moves, Shift as extending', () => {
        expect(keyIntent(key('ArrowDown'))).toEqual({ type: 'move', key: 'ArrowDown', extend: false })
        expect(keyIntent(key('End', { shiftKey: true }))).toEqual({ type: 'move', key: 'End', extend: true })
        expect(keyIntent(key('PageUp'))).toEqual({ type: 'move', key: 'PageUp', extend: false })
    })

    it('reads Space as toggle (Shift: range), Enter as open', () => {
        expect(keyIntent(key(' '))).toEqual({ type: 'toggle', range: false })
        expect(keyIntent(key(' ', { shiftKey: true }))).toEqual({ type: 'toggle', range: true })
        expect(keyIntent(key('Enter'))).toEqual({ type: 'open' })
    })

    it('opens the menu with the ContextMenu key and Shift+F10', () => {
        expect(keyIntent(key('ContextMenu'))).toEqual({ type: 'menu' })
        expect(keyIntent(key('F10', { shiftKey: true }))).toEqual({ type: 'menu' })
        expect(keyIntent(key('F10'))).toBeNull()
    })

    it('leaves global shortcuts and command-key combos alone', () => {
        expect(keyIntent(key('a', { metaKey: true }))).toBeNull()
        expect(keyIntent(key('Delete'))).toBeNull()
        expect(keyIntent(key('Escape'))).toBeNull()
        expect(keyIntent(key('ArrowLeft', { metaKey: true }))).toBeNull()
        expect(keyIntent(key('ArrowLeft', { altKey: true }))).toBeNull()
        expect(keyIntent(key('x'))).toBeNull()
    })
})

describe('navigate (grid of 4 columns, 10 books)', () => {
    //  0  1  2  3
    //  4  5  6  7
    //  8  9
    const go = (k: Parameters<typeof navigate>[0], from: number, pageRows = 1) => navigate(k, from, 10, 4, pageRows)

    it('moves left and right through the books, stopping at the ends', () => {
        expect(go('ArrowRight', 3)).toBe(4) // continues on the next row
        expect(go('ArrowLeft', 4)).toBe(3)
        expect(go('ArrowLeft', 0)).toBe(0)
        expect(go('ArrowRight', 9)).toBe(9)
    })

    it('moves up and down by a row, in the same column', () => {
        expect(go('ArrowDown', 1)).toBe(5)
        expect(go('ArrowUp', 5)).toBe(1)
        expect(go('ArrowUp', 2)).toBe(2) // first row: stays
    })

    it('goes down to the last book from above a shorter last row', () => {
        expect(go('ArrowDown', 6)).toBe(9)
        expect(go('ArrowDown', 9)).toBe(9)
        expect(go('ArrowDown', 8)).toBe(8)
    })

    it('jumps to the first and last books', () => {
        expect(go('Home', 6)).toBe(0)
        expect(go('End', 2)).toBe(9)
    })

    it('pages by whole screens of rows, keeping the column where it can', () => {
        expect(go('PageDown', 1, 1)).toBe(5)
        expect(go('PageDown', 1, 2)).toBe(9) // column 1 of the last row
        expect(go('PageDown', 3, 5)).toBe(9) // the last row has no column 3
        expect(go('PageUp', 9, 1)).toBe(5)
        expect(go('PageUp', 6, 5)).toBe(2) // same column, first row
    })

    it('handles a single-column list and an empty collection', () => {
        expect(navigate('ArrowDown', 2, 5, 1)).toBe(3)
        expect(navigate('ArrowRight', 2, 5, 1)).toBe(3)
        expect(navigate('ArrowDown', 0, 0, 4)).toBe(-1)
    })

    it('starts from the first (or last) book when nothing is active', () => {
        expect(navigate('ArrowDown', -1, 10, 4)).toBe(0)
        expect(navigate('End', -1, 10, 4)).toBe(9)
        expect(navigate('ArrowUp', -1, 10, 4)).toBe(9)
    })
})

describe('clickIntent', () => {
    const click = (mods: Partial<Record<'shiftKey' | 'ctrlKey' | 'metaKey', boolean>> = {}) =>
        ({ shiftKey: false, ctrlKey: false, metaKey: false, ...mods })

    it('opens on a plain click', () => {
        expect(clickIntent(click(), true)).toBe('open')
    })

    it('toggles with ⌘ on macOS and Ctrl elsewhere', () => {
        expect(clickIntent(click({ metaKey: true }), true)).toBe('toggle')
        expect(clickIntent(click({ ctrlKey: true }), true)).toBe('open') // macOS Ctrl-click is a right-click
        expect(clickIntent(click({ ctrlKey: true }), false)).toBe('toggle')
        expect(clickIntent(click({ metaKey: true }), false)).toBe('open')
    })

    it('selects a range with Shift, whatever else is held', () => {
        expect(clickIntent(click({ shiftKey: true }), true)).toBe('range')
        expect(clickIntent(click({ shiftKey: true, metaKey: true }), true)).toBe('range')
    })
})

describe('RangeSelection', () => {
    const ids = ['a', 'b', 'c', 'd', 'e', 'f']

    it('selects from the anchor to the target, in either direction', () => {
        const range = new RangeSelection()
        range.setAnchor('c', new Set())
        expect([...range.extend(ids, 4, new Set(), 0)].sort()).toEqual(['c', 'd', 'e'])
        const sel = range.extend(ids, 0, new Set(['c', 'd', 'e']), 0)
        expect([...sel].sort()).toEqual(['a', 'b', 'c'])
    })

    it('shrinks again when the target moves back (Shift+↓ then Shift+↑)', () => {
        const range = new RangeSelection()
        range.setAnchor('b', new Set())
        let sel = range.extend(ids, 3, new Set(), 0)
        expect([...sel].sort()).toEqual(['b', 'c', 'd'])
        sel = range.extend(ids, 2, sel, 0)
        expect([...sel].sort()).toEqual(['b', 'c'])
    })

    it('adds the range to what was selected when the anchor was set', () => {
        const range = new RangeSelection()
        const before = new Set(['f'])
        range.setAnchor('a', before)
        expect([...range.extend(ids, 1, before, 0)].sort()).toEqual(['a', 'b', 'f'])
    })

    it('takes an outside change (Clear, ⌘A) as the new base', () => {
        const range = new RangeSelection()
        range.setAnchor('a', new Set(['f']))
        range.extend(ids, 1, new Set(['f']), 0) // a, b, f
        // The user cleared the selection, then Shift+clicks again.
        const sel = range.extend(ids, 2, new Set(), 0)
        expect([...sel].sort()).toEqual(['a', 'b', 'c'])
    })

    it('uses the active book as the anchor when there is none (or it was filtered out)', () => {
        const range = new RangeSelection()
        expect([...range.extend(ids, 3, new Set(), 1)].sort()).toEqual(['b', 'c', 'd'])
        range.setAnchor('zzz', new Set())
        expect([...range.extend(ids, 0, new Set(), 2)].sort()).toEqual(['a', 'b', 'c'])
    })
})

describe('idsBetween and nextActiveId', () => {
    it('takes ids between two positions, inclusive and clamped', () => {
        expect(idsBetween(['a', 'b', 'c'], 2, 0)).toEqual(['a', 'b', 'c'])
        expect(idsBetween(['a', 'b', 'c'], 1, 1)).toEqual(['b'])
        expect(idsBetween(['a', 'b', 'c'], -5, 99)).toEqual(['a', 'b', 'c'])
        expect(idsBetween([], 0, 3)).toEqual([])
    })

    it('keeps the active book when it is still listed', () => {
        expect(nextActiveId(['a', 'b', 'c'], ['c', 'b', 'a'], 'b')).toBe('b')
    })

    it('moves to the book that took its place when it is gone', () => {
        expect(nextActiveId(['a', 'b', 'c', 'd'], ['a', 'c', 'd'], 'b')).toBe('c')
        expect(nextActiveId(['a', 'b', 'c'], ['a', 'b'], 'c')).toBe('b')
        expect(nextActiveId(['a'], [], 'a')).toBeNull()
        expect(nextActiveId(['a'], ['b'], null)).toBeNull()
    })
})
