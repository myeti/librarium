import { describe, expect, it } from 'vitest'
import { redactSecrets, REDACTED } from './redact'

describe('redactSecrets', () => {
    it('redacts a Google Books API key in a request URL', () => {
        const url = 'https://www.googleapis.com/books/v1/volumes?q=intitle%3ADune&maxResults=1&key=AIzaSyA-secret_123'
        const out = redactSecrets(url)
        expect(out).not.toContain('AIzaSyA-secret_123')
        expect(out).toBe(`https://www.googleapis.com/books/v1/volumes?q=intitle%3ADune&maxResults=1&key=${REDACTED}`)
    })

    it('keeps the parameters that follow the key', () => {
        const out = redactSecrets('https://x.test/a?key=SECRET&langRestrict=fr#top')
        expect(out).toBe(`https://x.test/a?key=${REDACTED}&langRestrict=fr#top`)
    })

    it('redacts keys inside a longer log line and several URLs', () => {
        const line = '[tidyup] query https://a.test/?key=ONE and https://b.test/?x=1&api_key=TWO done'
        const out = redactSecrets(line)
        expect(out).not.toMatch(/ONE|TWO/)
        expect(out).toContain('done')
    })

    it('handles other common secret names, case-insensitively', () => {
        expect(redactSecrets('?KEY=a')).toBe(`?KEY=${REDACTED}`)
        expect(redactSecrets('?access_token=a&b=1')).toBe(`?access_token=${REDACTED}&b=1`)
        expect(redactSecrets('&password=hunter2')).toBe(`&password=${REDACTED}`)
    })

    it('does not touch look-alike parameter names or plain text', () => {
        expect(redactSecrets('?monkey=banana')).toBe('?monkey=banana')
        expect(redactSecrets('the key=value pair')).toBe('the key=value pair')
    })
})
