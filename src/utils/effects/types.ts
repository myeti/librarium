/**
 * Types and level settings shared by the background effects
 * (`src/components/effects`). They live here rather than in the .vue files
 * because type-aware lint only sees a .vue import as a component (see
 * src/vite-env.d.ts).
 */
import type { ThemeName } from '@/stores/theme'

export type { ThemeName }

/**
 * How much background animation an effect shows. The user's "Off" setting is
 * not a level: the parent then doesn't render an effect at all.
 *
 * - `subtle`: about half the particles, the canvas drawn at most 30 times a
 *   second (motion is time-based, so things move at the same speed);
 * - `on`: every particle, drawn at the display's refresh rate.
 */
export type EffectLevel = 'subtle' | 'on'

export const EFFECT_LEVELS: readonly EffectLevel[] = ['subtle', 'on']

export function isEffectLevel(value: unknown): value is EffectLevel {
    return typeof value === 'string' && (EFFECT_LEVELS as readonly string[]).includes(value)
}

/** The canvas frame-rate cap of each level (`null`: the display's rate). */
export const LEVEL_MAX_FPS: Readonly<Record<EffectLevel, number | null>> = {
    subtle: 30,
    on: null,
}
