/**
 * Removes secrets from text before it is logged.
 *
 * The debug panel keeps every console line and offers "Copy all", so anything
 * logged can end up pasted in a bug report. Request URLs are logged for
 * troubleshooting, and some carry an API key in the query string (Google
 * Books: `?key=AIza…`). This keeps the URL readable but replaces the value.
 *
 * Matches `key=`, `api_key=`, `apikey=`, `access_token=`, `token=` and
 * `password=` as query parameters (after `?` or `&`), case-insensitively. The
 * value ends at the next `&`, `#`, whitespace or quote.
 */
const SECRET_PARAM = /([?&](?:key|api[_-]?key|access[_-]?token|token|password)=)[^&#\s"'<>]*/gi

export const REDACTED = '[redacted]'

export function redactSecrets(text: string): string {
    return text.replace(SECRET_PARAM, `$1${REDACTED}`)
}
