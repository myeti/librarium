import { describe, expect, it } from 'vitest'
import {
    anchorOffset, clampScrollTop, computeMetrics, firstFullyVisibleRow, maxScrollTop, restoreAnchor, rowAt,
    rowCount, rowOf, rowsPerPage, rowTop, scrollTopToReveal, totalHeight, visibleRows,
    type GridLayout, type ListLayout,
} from './virtualWindow'

const GRID: GridLayout = { kind: 'grid', minColumnWidth: 180, gap: 16, padding: 20, infoHeight: 90, coverRatio: 1.6 }
const LIST: ListLayout = { kind: 'list', rowHeight: 68, paddingY: 4, paddingX: 20 }

describe('computeMetrics', () => {
    it('fits as many columns of the minimum width as the width allows', () => {
        // 1000 − 2×20 = 960 inner: 4 × 180 + 3 × 16 = 768 fits, 5 × 180 + 4 × 16 = 964 doesn't.
        const m = computeMetrics(GRID, 1000)
        expect(m.columns).toBe(4)
        expect(m.columnWidth).toBe((960 - 3 * 16) / 4)
        expect(m.itemHeight).toBeCloseTo(m.columnWidth * 1.6 + 90)
        expect(m.rowStride).toBeCloseTo(m.itemHeight + 16)
    })

    it('goes from 4 to 5 columns exactly when the fifth fits', () => {
        expect(computeMetrics(GRID, 2 * 20 + 5 * 180 + 4 * 16 - 1).columns).toBe(4)
        expect(computeMetrics(GRID, 2 * 20 + 5 * 180 + 4 * 16).columns).toBe(5)
    })

    it('keeps at least one column, even narrower than the minimum (or before measuring)', () => {
        expect(computeMetrics(GRID, 100).columns).toBe(1)
        const unmeasured = computeMetrics(GRID, 0)
        expect(unmeasured.columns).toBe(1)
        expect(unmeasured.columnWidth).toBe(0)
        expect(unmeasured.itemHeight).toBe(90)
    })

    it('lays a list out as one column of fixed rows without gaps', () => {
        const m = computeMetrics(LIST, 800)
        expect(m).toMatchObject({ columns: 1, columnWidth: 760, itemHeight: 68, rowStride: 68, gap: 0, paddingTop: 4, paddingBottom: 4 })
    })
})

describe('rows and heights', () => {
    const m = computeMetrics(LIST, 800)

    it('counts rows, rounding the last partial row up', () => {
        expect(rowCount(0, 4)).toBe(0)
        expect(rowCount(1, 4)).toBe(1)
        expect(rowCount(8, 4)).toBe(2)
        expect(rowCount(9, 4)).toBe(3)
        expect(rowOf(0, 4)).toBe(0)
        expect(rowOf(7, 4)).toBe(1)
        expect(rowOf(8, 4)).toBe(2)
    })

    it('measures the canvas: padding, rows, no gap after the last row', () => {
        expect(totalHeight(m, 0)).toBe(0)
        expect(totalHeight(m, 10)).toBe(4 + 10 * 68 + 4)
        const g = computeMetrics(GRID, 1000)
        expect(totalHeight(g, 8)).toBeCloseTo(20 + 2 * g.rowStride - 16 + 20)
    })

    it('places rows and finds the row under a position', () => {
        expect(rowTop(m, 0)).toBe(4)
        expect(rowTop(m, 3)).toBe(4 + 3 * 68)
        expect(rowAt(m, 10, 0)).toBe(0)
        expect(rowAt(m, 10, 4 + 68 * 2 + 1)).toBe(2)
        expect(rowAt(m, 10, 1e6)).toBe(9)
        expect(rowAt(m, 0, 100)).toBe(0)
    })

    it('clamps scroll positions to the canvas', () => {
        expect(maxScrollTop(m, 10, 300)).toBe(4 + 680 + 4 - 300)
        expect(maxScrollTop(m, 2, 300)).toBe(0)
        expect(clampScrollTop(m, 10, 300, -50)).toBe(0)
        expect(clampScrollTop(m, 10, 300, 1e6)).toBe(388)
    })
})

describe('visibleRows', () => {
    const m = computeMetrics(LIST, 800)

    it('renders the rows in the viewport, widened by the overscan', () => {
        // 300 px viewport at scrollTop 680: rows 9 (676…744) to 14 (956…1024).
        expect(visibleRows(m, 1000, 680, 300)).toEqual({ first: 9, last: 14 })
        expect(visibleRows(m, 1000, 680, 300, 2)).toEqual({ first: 7, last: 16 })
    })

    it('never goes past the first or last row', () => {
        expect(visibleRows(m, 1000, 0, 300, 5)).toEqual({ first: 0, last: 9 })
        expect(visibleRows(m, 20, 1e6, 300, 2)).toEqual({ first: 17, last: 19 })
    })

    it('is empty without books', () => {
        const r = visibleRows(m, 0, 0, 300, 2)
        expect(r.last).toBeLessThan(r.first)
    })

    it('renders a bounded number of rows whatever the library size', () => {
        const big = visibleRows(m, 100_000, 3_000_000, 900, 2)
        expect(big.last - big.first + 1).toBeLessThanOrEqual(Math.ceil(900 / 68) + 1 + 4)
    })

    it('finds the first row that is entirely on screen', () => {
        expect(firstFullyVisibleRow(m, 100, 0, 300)).toBe(0)
        // Row 2 starts at 140; scrolled to 150 it is cut, so row 3 is the first whole one.
        expect(firstFullyVisibleRow(m, 100, 150, 300)).toBe(3)
        // A row taller than the viewport: the one at the top.
        expect(firstFullyVisibleRow(m, 100, 150, 40)).toBe(2)
    })
})

describe('scrollTopToReveal', () => {
    const m = computeMetrics(LIST, 800) // rows of 68 from y = 4

    it('leaves the scroll alone when the item is entirely visible', () => {
        expect(scrollTopToReveal(m, 100, 3, 0, 400)).toBe(0)
    })

    it('scrolls down just enough to show an item below, with a margin', () => {
        // Row 10: 684…752; viewport 400 → bottom at 752 + margin (4) − 400.
        expect(scrollTopToReveal(m, 100, 10, 0, 400)).toBe(752 + 4 - 400)
    })

    it('scrolls up to an item above, and to the very top for the first row', () => {
        expect(scrollTopToReveal(m, 100, 5, 1000, 400)).toBe(rowTop(m, 5) - 4)
        expect(scrollTopToReveal(m, 100, 0, 1000, 400)).toBe(0)
    })

    it('shows the last row with the bottom padding, within the canvas', () => {
        const target = scrollTopToReveal(m, 100, 99, 0, 400)
        expect(target).toBe(maxScrollTop(m, 100, 400))
    })

    it('aligns an item taller than the viewport on its top', () => {
        const g = computeMetrics(GRID, 1000) // cards ~400 px tall
        const target = scrollTopToReveal(g, 100, 8, 0, 300)
        expect(target).toBe(rowTop(g, 2) - 16)
    })

    it('works on grid rows (the row of the item counts)', () => {
        const g = computeMetrics(GRID, 1000) // 4 columns
        expect(scrollTopToReveal(g, 100, 5, 0, 2000)).toBe(0) // row 1 already visible
        expect(scrollTopToReveal(g, 100, 40, 0, 800)).toBeGreaterThan(0)
    })
})

describe('paging and anchoring', () => {
    it('pages by the whole rows that fit', () => {
        const m = computeMetrics(LIST, 800)
        expect(rowsPerPage(m, 680)).toBe(10)
        expect(rowsPerPage(m, 10)).toBe(1)
    })

    it('puts the anchor book back at the same offset after the list changed', () => {
        const m = computeMetrics(LIST, 800)
        const scrollTop = 1000
        const offset = anchorOffset(m, 20, scrollTop) // book 20 sat here
        // Sorted again: the same book is now 50th.
        const next = restoreAnchor(m, 200, 400, 50, offset)
        expect(rowTop(m, 50) - next).toBe(offset)
    })

    it('keeps the anchor book in place when the number of columns changes', () => {
        const wide = computeMetrics(GRID, 1400)  // 6 columns
        const narrow = computeMetrics(GRID, 900) // 4 columns
        const scrollTop = 2000
        const offset = anchorOffset(wide, 30, scrollTop)
        const next = restoreAnchor(narrow, 300, 700, 30, offset)
        expect(rowTop(narrow, rowOf(30, narrow.columns)) - next).toBeCloseTo(offset)
    })

    it('clamps when the anchor would need scrolling past the ends', () => {
        const m = computeMetrics(LIST, 800)
        expect(restoreAnchor(m, 5, 400, 0, 200)).toBe(0)
        expect(restoreAnchor(m, 20, 400, 19, -10_000)).toBe(maxScrollTop(m, 20, 400))
    })
})
