/**
 * Virtual scrolling maths for the library grid and list.
 *
 * A library can hold thousands of books; rendering a card for each one
 * would mount thousands of components. The collection renders only the rows
 * that are on screen, plus a few above and below ("overscan"), inside a
 * canvas as tall as all the rows would be. Rows are absolutely positioned at
 * `rowTop(row)`.
 *
 * Every item has the same size, so nothing has to be measured but the
 * scroller itself:
 * - **grid**: as many columns of at least `minColumnWidth` as fit; a card is
 *   a cover (`coverRatio` × the column width) plus a text block of fixed
 *   height (`infoHeight`), so a row's height follows from the width;
 * - **list**: one column of rows of `rowHeight`.
 *
 * Everything here is a pure function of numbers (tested in
 * virtualWindow.test.ts); BookCollection.vue feeds it the scroller's size
 * and scroll position.
 */

export interface GridLayout {
    kind: 'grid'
    /** The narrowest a column may be, px. */
    minColumnWidth: number
    /** Gap between cards, horizontally and vertically, px. */
    gap: number
    /** Space around the cards (all four sides), px. */
    padding: number
    /** Height of the text block under each cover, px. */
    infoHeight: number
    /** Cover height divided by cover width. */
    coverRatio: number
}

export interface ListLayout {
    kind: 'list'
    /** Height of one row, px. */
    rowHeight: number
    /** Space above the first row and below the last one, px. */
    paddingY: number
    /** Space left and right of the rows, px. */
    paddingX: number
}

export type CollectionLayout = GridLayout | ListLayout

/** The sizes the collection is laid out with, for one scroller width. */
export interface Metrics {
    columns: number
    columnWidth: number
    /** Height of one item (card or row), px. */
    itemHeight: number
    /** Distance from one row's top to the next one's (item height + gap). */
    rowStride: number
    /** Gap between rows (0 in a list). */
    gap: number
    paddingTop: number
    paddingBottom: number
    /** Horizontal padding (each side). */
    paddingX: number
}

/** Rows to render, inclusive. Empty (`last < first`) when there is nothing to show. */
export interface RowRange {
    first: number
    last: number
}

export function computeMetrics(layout: CollectionLayout, width: number): Metrics {
    if (layout.kind === 'list') {
        return {
            columns: 1,
            columnWidth: Math.max(0, width - 2 * layout.paddingX),
            itemHeight: layout.rowHeight,
            rowStride: layout.rowHeight,
            gap: 0,
            paddingTop: layout.paddingY,
            paddingBottom: layout.paddingY,
            paddingX: layout.paddingX,
        }
    }
    const inner = Math.max(0, width - 2 * layout.padding)
    // n columns need n × min + (n − 1) × gap ≤ inner.
    const columns = Math.max(1, Math.floor((inner + layout.gap) / (layout.minColumnWidth + layout.gap)))
    const columnWidth = Math.max(0, (inner - (columns - 1) * layout.gap) / columns)
    const itemHeight = columnWidth * layout.coverRatio + layout.infoHeight
    return {
        columns,
        columnWidth,
        itemHeight,
        rowStride: itemHeight + layout.gap,
        gap: layout.gap,
        paddingTop: layout.padding,
        paddingBottom: layout.padding,
        paddingX: layout.padding,
    }
}

export function rowCount(count: number, columns: number): number {
    return count <= 0 ? 0 : Math.ceil(count / Math.max(1, columns))
}

/** The row holding item `index`. */
export function rowOf(index: number, columns: number): number {
    return Math.floor(Math.max(0, index) / Math.max(1, columns))
}

/** Where row `row` starts, from the top of the canvas. */
export function rowTop(m: Metrics, row: number): number {
    return m.paddingTop + row * m.rowStride
}

/** Height of the whole canvas for `count` items. */
export function totalHeight(m: Metrics, count: number): number {
    const rows = rowCount(count, m.columns)
    if (rows === 0) { return 0 }
    return m.paddingTop + rows * m.rowStride - m.gap + m.paddingBottom
}

/** The highest scroll position for `count` items in a viewport of `viewportHeight`. */
export function maxScrollTop(m: Metrics, count: number, viewportHeight: number): number {
    return Math.max(0, totalHeight(m, count) - viewportHeight)
}

export function clampScrollTop(m: Metrics, count: number, viewportHeight: number, scrollTop: number): number {
    return Math.min(Math.max(0, scrollTop), maxScrollTop(m, count, viewportHeight))
}

/** The row under the canvas position `y` (clamped to the existing rows; 0 when there are none). */
export function rowAt(m: Metrics, count: number, y: number): number {
    const rows = rowCount(count, m.columns)
    if (rows === 0) { return 0 }
    const row = Math.floor((y - m.paddingTop) / m.rowStride)
    return Math.min(Math.max(0, row), rows - 1)
}

/**
 * The rows intersecting the viewport, widened by `overscan` rows on each
 * side so a fast scroll doesn't show blank space before the next render.
 * `scrollTop` is measured from the top of the canvas.
 */
export function visibleRows(m: Metrics, count: number, scrollTop: number, viewportHeight: number, overscan = 0): RowRange {
    const rows = rowCount(count, m.columns)
    if (rows === 0) { return { first: 0, last: -1 } }
    const top = rowAt(m, count, scrollTop)
    const bottom = rowAt(m, count, scrollTop + Math.max(0, viewportHeight))
    return {
        first: Math.max(0, top - overscan),
        last: Math.min(rows - 1, bottom + overscan),
    }
}

/** The first row entirely inside the viewport (the top one when none is: a row taller than the viewport). */
export function firstFullyVisibleRow(m: Metrics, count: number, scrollTop: number, viewportHeight: number): number {
    const row = rowAt(m, count, scrollTop)
    const rows = rowCount(count, m.columns)
    const partlyHidden = rowTop(m, row) < scrollTop
    const nextFits = rowTop(m, row + 1) + m.itemHeight <= scrollTop + viewportHeight
    return partlyHidden && row + 1 < rows && nextFits ? row + 1 : row
}

/**
 * The scroll position that brings item `index` into view, moving as little
 * as possible: unchanged if it is already entirely visible, else just
 * enough to show it with a small margin (the padding for the first and
 * last rows). An item taller than the viewport is aligned on its top.
 */
export function scrollTopToReveal(m: Metrics, count: number, index: number, scrollTop: number, viewportHeight: number): number {
    const row = rowOf(index, m.columns)
    const top = rowTop(m, row)
    const bottom = top + m.itemHeight
    const rows = rowCount(count, m.columns)
    const marginTop = row === 0 ? m.paddingTop : Math.min(m.gap || 8, m.paddingTop)
    const marginBottom = row === rows - 1 ? m.paddingBottom : Math.min(m.gap || 8, m.paddingBottom)
    let next = scrollTop
    if (top < scrollTop) {
        next = top - marginTop
    } else if (bottom > scrollTop + viewportHeight) {
        const tooTall = m.itemHeight + marginTop + marginBottom > viewportHeight
        next = tooTall ? top - marginTop : bottom + marginBottom - viewportHeight
    }
    return clampScrollTop(m, count, viewportHeight, next)
}

/** How many whole rows fit in the viewport (at least 1): what PageUp / PageDown move by. */
export function rowsPerPage(m: Metrics, viewportHeight: number): number {
    return Math.max(1, Math.floor((viewportHeight + m.gap) / m.rowStride))
}

/**
 * Scroll anchoring: where an item's row was relative to the viewport
 * (`rowTop − scrollTop`), so the same item can be put back in the same
 * place after the list or the layout changed (`restoreAnchor`).
 */
export function anchorOffset(m: Metrics, index: number, scrollTop: number): number {
    return rowTop(m, rowOf(index, m.columns)) - scrollTop
}

/** The scroll position that puts item `index` back at `offset` from the viewport top (clamped). */
export function restoreAnchor(m: Metrics, count: number, viewportHeight: number, index: number, offset: number): number {
    return clampScrollTop(m, count, viewportHeight, rowTop(m, rowOf(index, m.columns)) - offset)
}
