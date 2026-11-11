import { beforeEach, describe, expect, it } from 'vitest'
import { avatarColor, initials, resetAvatarColors } from './avatar'
import { COVER_COLOR_COUNT } from './coverArt'

/** A seeded generator, so a deal is the same on every run. */
function seeded(seed: number): () => number {
    let s = seed
    return () => {
        s = (s * 16807) % 2147483647
        return (s - 1) / 2147483646
    }
}

const users = (n: number) => Array.from({ length: n }, (_, i) => `user ${i}`)

beforeEach(() => resetAvatarColors(seeded(1)))

describe('avatar colours', () => {
    it('dress a user in one of the theme\'s cover cloths, the same all session', () => {
        // Their initials use --cover-ink, checked on every cloth by themes.test.ts.
        expect(avatarColor('Alice')).toMatch(/^var\(--cover-[1-6]\)$/)
        expect(avatarColor('Alice')).toBe(avatarColor('Alice'))
    })

    it('give every user of a group of six a different colour, then deal all six again', () => {
        const colors = users(2 * COVER_COLOR_COUNT + 1).map(avatarColor)
        const first = colors.slice(0, COVER_COLOR_COUNT)
        const second = colors.slice(COVER_COLOR_COUNT, 2 * COVER_COLOR_COUNT)
        expect(new Set(first).size).toBe(COVER_COLOR_COUNT)
        expect(new Set(second).size).toBe(COVER_COLOR_COUNT)
        expect(colors[2 * COVER_COLOR_COUNT]).toMatch(/^var\(--cover-[1-6]\)$/)
    })

    it('deal again at the next launch: the colours change', () => {
        const deal = (seed: number) => {
            resetAvatarColors(seeded(seed))
            return users(COVER_COLOR_COUNT).map(avatarColor).join()
        }
        const deals = new Set([1, 2, 3, 4, 5].map(deal))
        expect(deals.size).toBeGreaterThan(1)
    })

    it('take up to two initials', () => {
        expect(initials('Jean Paul Sartre')).toBe('JP')
        expect(initials('émilie')).toBe('É')
    })
})
