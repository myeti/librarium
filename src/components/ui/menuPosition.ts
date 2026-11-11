import type { MenuPlacement } from './types'

/** A box in viewport coordinates (what getBoundingClientRect returns). */
export interface Box {
    left: number
    top: number
    width: number
    height: number
}

export interface Viewport {
    width: number
    height: number
}

/** Gap kept between a menu and the window edges. */
export const VIEWPORT_MARGIN = 8

/**
 * Where to put a menu of size `menu` so it stays inside the window.
 *
 * - Anchored to an element (`anchor` is a box): below it (or above for
 *   `top-*`), aligned to its start or end edge; flipped to the other side
 *   when it doesn't fit, then clamped into the window.
 * - At a point (a context menu: `anchor` has zero size): the menu's corner
 *   at the point, flipped left/up when it would overflow, then clamped.
 *
 * Returns the top-left corner for `position: fixed`.
 */
export function computeMenuPosition(
    anchor: Box,
    menu: { width: number, height: number },
    viewport: Viewport,
    placement: MenuPlacement = 'bottom-start',
    offset = 4,
): { left: number, top: number } {
    const isPoint = anchor.width === 0 && anchor.height === 0
    const gap = isPoint ? 0 : offset
    const anchorRight = anchor.left + anchor.width
    const anchorBottom = anchor.top + anchor.height

    // Horizontal: start = menu's left edge on the anchor's left edge.
    const wantsEnd = placement.endsWith('-end')
    let left = wantsEnd ? anchorRight - menu.width : anchor.left
    if (isPoint && left + menu.width > viewport.width - VIEWPORT_MARGIN) {
        left = anchor.left - menu.width
    }

    // Vertical: below, unless asked for above; flip when that side lacks room.
    const below = anchorBottom + gap
    const above = anchor.top - gap - menu.height
    const fitsBelow = below + menu.height <= viewport.height - VIEWPORT_MARGIN
    const fitsAbove = above >= VIEWPORT_MARGIN
    let top: number
    if (placement.startsWith('top')) {
        top = fitsAbove || !fitsBelow ? above : below
    } else {
        top = fitsBelow || !fitsAbove ? below : above
    }

    return {
        left: clamp(left, VIEWPORT_MARGIN, viewport.width - menu.width - VIEWPORT_MARGIN),
        top: clamp(top, VIEWPORT_MARGIN, viewport.height - menu.height - VIEWPORT_MARGIN),
    }
}

/** Clamps into [min, max]; when the range is empty (menu larger than the window), `min` wins. */
function clamp(value: number, min: number, max: number): number {
    return Math.max(min, Math.min(value, max))
}
