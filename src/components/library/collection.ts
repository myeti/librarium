/**
 * Types, constants and small pure helpers shared by the library screen's
 * components (src/components/library). They live in a .ts module because
 * type-aware lint sees a .vue import only as a component (see
 * src/vite-env.d.ts).
 */
import type { ShelfItem } from '@/services/shelf'
import type { GridLayout, ListLayout } from '@/utils/virtualWindow'

/** Asked of the view when an item's menu should open (right-click, ContextMenu key, Shift+F10). */
export interface CollectionMenuRequest {
    item: ShelfItem
    /** Viewport coordinates for the menu's corner. */
    x: number
    y: number
    /** The book's element: focus goes back to it when the menu closes. */
    origin: HTMLElement
}

/**
 * The grid: cards of at least 170 px, 26 px apart (books on a shelf, with
 * room for a hovered one to lift and glow), 28 px from the edges. Under each
 * cover, a text block of 92 px: padding (11 + 4) + a series line (17) + two
 * title lines (38) + 2 + the author (17) + 3 spare. The cover ratio is the
 * `--cover-ratio` token (2 / 3.2). BookCard's CSS lays out that block.
 */
export const GRID_LAYOUT: Readonly<GridLayout> = Object.freeze({
    kind: 'grid',
    minColumnWidth: 170,
    gap: 26,
    padding: 28,
    infoHeight: 92,
    coverRatio: 3.2 / 2,
})

/** The list: rows of 68 px (a 36 px cover is 57.6 px tall). */
export const LIST_LAYOUT: Readonly<ListLayout> = Object.freeze({
    kind: 'list',
    rowHeight: 68,
    paddingY: 4,
    paddingX: 20,
})

/**
 * The width of a panel docked on the right of a resize handle, after a drag
 * from `startX` to `x` (dragging left widens it), clamped to [min, max].
 */
export function draggedWidth(startWidth: number, startX: number, x: number, min: number, max: number): number {
    return Math.min(max, Math.max(min, startWidth + (startX - x)))
}

/** `items` with the item at `from` moved to `to` (clamped to the list): a reorder. */
export function moved<T>(items: readonly T[], from: number, to: number): T[] {
    const next = [...items]
    const [item] = next.splice(from, 1)
    next.splice(Math.max(0, Math.min(next.length, to)), 0, item)
    return next
}
