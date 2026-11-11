/**
 * The "flying cover" shown when books are sent to the e-reader: the book's
 * cover is cloned from the grid/list and animated into the sidebar's device
 * box. Purely decorative: callers fire and forget, and it never throws.
 *
 * Steps:
 *  1. Locate the cover element via [data-book-id] and the device target
 *  2. Clone the cover, fix-position it over the source
 *  3. Keyframe: lift slightly → arc across → shrink into the device box
 *  4. Give the device box a brief "thud" pulse on landing
 */
export function flyToDevice(bookId: string): void {
    animateFly(bookId).catch(() => {
        // An animation cancelled half-way (the card or the sidebar went away): nothing to do.
    })
}

async function animateFly(bookId: string): Promise<void> {
    const cardEl   = document.querySelector(`[data-book-id="${CSS.escape(bookId)}"]`)
    const coverEl  = cardEl?.querySelector<HTMLElement>('.book-cover') ?? null
    const targetEl = document.querySelector<HTMLElement>('.sidebar_device')

    if (!coverEl || !targetEl) { return }

    const src = coverEl.getBoundingClientRect()
    const tgt = targetEl.getBoundingClientRect()

    // Skip if the card is scrolled out of the viewport.
    if (src.bottom < 0 || src.top > window.innerHeight) { return }

    // --- Build the shadow clone -----------------------------------------------
    const clone = coverEl.cloneNode(true) as HTMLElement

    // Reset all layout properties and fix it exactly over the source cover.
    Object.assign(clone.style, {
        position:      'fixed',
        left:          `${src.left}px`,
        top:           `${src.top}px`,
        width:         `${src.width}px`,
        height:        `${src.height}px`,
        margin:        '0',
        padding:       '0',
        pointerEvents: 'none',
        zIndex:        '9999',
        borderRadius:  '8px',
        overflow:      'hidden',
        boxShadow:     '0 16px 48px rgba(0, 0, 0, 0.45)',
        transformOrigin: '50% 50%',
    })

    document.body.appendChild(clone)
    try {
        await fly(clone, src, tgt)
    } finally {
        clone.remove()
    }
    // Brief "thud" pulse on the device box so the user feels the landing.
    targetEl.animate(
        [
            { transform: 'scale(1)',    offset: 0   },
            { transform: 'scale(1.06)', offset: 0.3 },
            { transform: 'scale(1)',    offset: 1   },
        ],
        { duration: 350, easing: 'cubic-bezier(0.36, 0.07, 0.19, 0.97)' },
    )
}

/** Moves `clone` from the `src` rectangle to the centre of `tgt`, shrinking it away. */
async function fly(clone: HTMLElement, src: DOMRect, tgt: DOMRect): Promise<void> {
    // --- Compute translation from source centre to target centre ---------------
    const srcCx = src.left  + src.width  / 2
    const srcCy = src.top   + src.height / 2
    const tgtCx = tgt.left  + tgt.width  / 2
    const tgtCy = tgt.top   + tgt.height / 2

    const dx = tgtCx - srcCx
    const dy = tgtCy - srcCy

    // --- Fly animation ---------------------------------------------------------
    // Phase 1 (0–30 %): lift up and grow slightly — "jumping" feel.
    // Phase 2 (30–100 %): arc toward the device box and shrink to nothing.
    const jump = clone.animate([{
        transform: 'translate(0, 0) scale(1)',
        offset:    0,
    }, {
        transform: `translate(${dx * 0.2}px, -50px) scale(1.4)`,
        offset:    1,
    }], {
        duration: 300,
        easing:   'ease-out',
        fill:     'forwards',
    })

    await jump.finished

    const flight = clone.animate([{
        transform: `translate(${dx * 0.2}px, -50px) scale(1.4)`,
        opacity:   '1',
        offset:    0,
    }, {
        transform: `translate(${dx}px, ${dy}px) scale(0.08)`,
        opacity:   '0',
        offset:    1,
    }], {
        duration: 350,
        easing:   'ease-in',
        fill:     'forwards',
    })

    await flight.finished
}
