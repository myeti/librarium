/**
 * The random installation ID: the only identifier Librarium sends, with
 * error reports and feedback (PRIVACY.md). It lets the developer see that
 * several reports come from the same installation; it says nothing about
 * who you are.
 *
 * Its life (decision D2):
 * - **Created only when needed:** by the first error report (which needs the
 *   statistics consent) or by feedback (which the user sends), never at
 *   startup and never for usage events (those carry no identifier).
 * - **Deleted** when the user says no to statistics (the consent dialog's
 *   "No thanks", or the switch in Settings), and at startup while no choice
 *   is recorded: an ID from before the consent existed is never reused.
 *   See `stores/analytics.ts`.
 *
 * Kept in the web view's localStorage under its historical key `appUuid`.
 * (Versions up to 0.5 also wrote an unrelated `appUid` into the settings
 * file, which was never sent; the analytics store clears it.)
 */
export const INSTALLATION_ID_KEY = 'appUuid'

/** The ID, or null when there is none (nothing has needed it yet). */
export function peekInstallationId(): string | null {
    try {
        return localStorage.getItem(INSTALLATION_ID_KEY)
    } catch {
        return null
    }
}

/** The ID, created and saved now if there is none yet. */
export function getInstallationId(): string {
    const existing = peekInstallationId()
    if (existing) { return existing }
    const id = crypto.randomUUID()
    try {
        localStorage.setItem(INSTALLATION_ID_KEY, id)
    } catch {
        // Storage unavailable: this report goes with a one-off ID.
    }
    return id
}

/** Deletes the ID; the next report or feedback gets a new, unrelated one. */
export function resetInstallationId(): void {
    try {
        localStorage.removeItem(INSTALLATION_ID_KEY)
    } catch {
        // Nothing stored, nothing to delete.
    }
}
