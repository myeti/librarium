import { uniqueId } from '@/utils/dom'

/** The ids a form field links together: control, hint and error. */
export interface FieldIds {
    controlId: string
    hintId: string
    errorId: string
}

/** Fresh ids for a field; `id` (when given) is used for the control. */
export function fieldIds(prefix: string, id = ''): FieldIds {
    const base = id || uniqueId(prefix)
    return { controlId: base, hintId: `${base}-hint`, errorId: `${base}-error` }
}

/** The `aria-describedby` value of a field's control (undefined when there is nothing to describe). */
export function describedBy(ids: FieldIds, hint: string, error: string): string | undefined {
    const parts = [hint ? ids.hintId : '', error ? ids.errorId : ''].filter(Boolean)
    return parts.length ? parts.join(' ') : undefined
}
