/**
 * Tells components when their element enters or leaves the screen, with one
 * shared IntersectionObserver for the whole app (one per card would cost
 * thousands of observers on a large library). Used to load covers only for
 * the cards the user can see.
 */

export type VisibilityCallback = (visible: boolean) => void

/** Start loading a little before a card scrolls into view. */
const ROOT_MARGIN = '300px 0px'

let observer: IntersectionObserver | null = null
const callbacks = new Map<Element, VisibilityCallback>()

function sharedObserver(): IntersectionObserver {
    observer ??= new IntersectionObserver(entries => {
        for (const entry of entries) {
            callbacks.get(entry.target)?.(entry.isIntersecting)
        }
    }, { rootMargin: ROOT_MARGIN })
    return observer
}

/**
 * Calls `callback(true)` when `el` becomes visible and `callback(false)`
 * when it leaves, until the returned function is called. Where
 * IntersectionObserver doesn't exist, `el` counts as visible at once.
 */
export function watchVisibility(el: Element, callback: VisibilityCallback): () => void {
    if (typeof IntersectionObserver === 'undefined') {
        callback(true)
        return () => {}
    }
    callbacks.set(el, callback)
    sharedObserver().observe(el)
    return () => {
        callbacks.delete(el)
        observer?.unobserve(el)
    }
}
