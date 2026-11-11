/**
 * Small DOM helpers shared by the UI kit (src/components/ui) and the
 * shortcuts store: focus management, typing detection, unique ids.
 */

/** Elements that can take keyboard focus (before checking visibility). */
export const FOCUSABLE_SELECTOR = [
    'a[href]',
    'button:not([disabled])',
    'input:not([disabled]):not([type="hidden"])',
    'select:not([disabled])',
    'textarea:not([disabled])',
    'iframe',
    '[contenteditable=""], [contenteditable="true"]',
    '[tabindex]',
].join(', ')

/**
 * The keyboard-focusable elements inside `root`, in DOM order: enabled,
 * not taken out of the tab order (`tabindex="-1"`), not inside an `inert`
 * or `hidden` subtree. Layout isn't checked (happy-dom has none); a
 * `display: none` element is skipped by the browser's own focus() anyway.
 */
export function focusableWithin(root: ParentNode): HTMLElement[] {
    return Array.from(root.querySelectorAll<HTMLElement>(FOCUSABLE_SELECTOR))
        .filter(el => {
            // The attribute, not el.tabIndex: happy-dom reports -1 for a plain <button>.
            const tabindex = el.getAttribute('tabindex')
            return (tabindex === null || Number(tabindex) >= 0) && !el.closest('[inert], [hidden]')
        })
}

/**
 * Whether keys pressed while `el` has focus are text input: a text field,
 * a select, or editable content. Global shortcuts ignore those keys, so
 * typing "a" in the search field never triggers "select all".
 */
export function isTypingTarget(el: EventTarget | Element | null): boolean {
    if (el instanceof HTMLInputElement) {
        // Checkboxes, radios and buttons don't take text.
        return !['checkbox', 'radio', 'button', 'submit', 'reset', 'range', 'color', 'file'].includes(el.type)
    }
    return el instanceof HTMLTextAreaElement
        || el instanceof HTMLSelectElement
        || (el instanceof HTMLElement && el.isContentEditable)
}

let counter = 0

/**
 * A unique id for linking elements (`<label for>`, `aria-labelledby`,
 * `aria-describedby`). Ids only need to be unique within the page, so a
 * counter is enough; the prefix makes them readable in the inspector.
 */
export function uniqueId(prefix = 'ui'): string {
    counter += 1
    return `${prefix}-${counter}`
}
