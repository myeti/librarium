/**
 * Types of the UI kit's props. They live here rather than in the .vue files
 * because type-aware lint only sees a .vue import as a component (see
 * src/vite-env.d.ts).
 */

export type ButtonVariant = 'primary' | 'secondary' | 'ghost' | 'danger'
export type ButtonSize = 'sm' | 'md'

export type ModalSize = 'sm' | 'md' | 'lg' | 'xl' | 'full' | 'screen'
/** Why a dialog asks to be closed (BaseModal's `close` event). */
export type ModalCloseReason = 'escape' | 'backdrop' | 'button'

export type MenuPlacement = 'bottom-start' | 'bottom-end' | 'top-start' | 'top-end'

export type BadgeTone = 'neutral' | 'accent' | 'success' | 'warning' | 'danger'
    | 'epub' | 'pdf' | 'mobi' | 'azw3' | 'cbz'

export type ProgressTone = 'accent' | 'success' | 'danger'

export interface SelectOption {
    value: string | number
    label: string
    disabled?: boolean
}
