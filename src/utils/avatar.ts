import { COVER_COLOR_COUNT } from '@/utils/coverArt'

/** Returns up to two uppercase initials from a display name. */
export function initials(name: string): string {
    return name
        .split(/\s+/)
        .slice(0, 2)
        .map(w => w[0]?.toUpperCase() ?? '')
        .join('')
}

/** A source of numbers in [0, 1) (Math.random; seeded in tests). */
type Random = () => number

/**
 * The colours given out this session: a shuffled deck of the cover cloths,
 * dealt one per new user; when it is empty, all the cloths are shuffled
 * again. So any run of COVER_COLOR_COUNT users gets different colours, and
 * the colours change from one launch to the next.
 */
class AvatarDeck {
    private readonly given = new Map<string, number>()
    private deck: number[] = []

    constructor(private readonly random: Random) {}

    /** The cloth (1 to COVER_COLOR_COUNT) of `name`: dealt the first time, the same after. */
    colorOf(name: string): number {
        let color = this.given.get(name)
        if (color === undefined) {
            if (this.deck.length === 0) { this.deck = this.shuffled() }
            color = this.deck.pop()!
            this.given.set(name, color)
        }
        return color
    }

    /** Every cloth, in a random order (Fisher–Yates). */
    private shuffled(): number[] {
        const colors = Array.from({ length: COVER_COLOR_COUNT }, (_, i) => i + 1)
        for (let i = colors.length - 1; i > 0; i--) {
            const j = Math.floor(this.random() * (i + 1))
            ;[colors[i], colors[j]] = [colors[j], colors[i]]
        }
        return colors
    }
}

let avatars = new AvatarDeck(Math.random)

/**
 * A user's avatar colour for this session, the same everywhere (user
 * picker, sidebar): one of the theme's cover cloths, `var(--cover-N)` (the
 * soft jewel tones of the generated covers), under initials in
 * `--cover-ink`, which themes.test.ts checks is readable on every cloth.
 * Dealt at random from a deck (see AvatarDeck): six users in a row never
 * share a colour; the next launch deals again.
 */
export function avatarColor(name: string): string {
    return `var(--cover-${avatars.colorOf(name)})`
}

/** Tests: a new session's deck, shuffled with `random`. */
export function resetAvatarColors(random: Random = Math.random): void {
    avatars = new AvatarDeck(random)
}
