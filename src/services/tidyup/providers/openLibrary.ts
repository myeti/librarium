import type { TidyupProvider, TidyupQueryResult, QueryParams } from '../types'
import { detectCategory } from '@/constants/categories'
import { stripHtml } from '../utils'
import { stripUndefined, withoutPlaceholderCover } from '../merge'
import { fetchJson } from '../fetchJson'
import { isbnLookupForms } from '@/utils/isbn'

const LABEL = 'Open Library'

/** Open Library writes a text either as a string or as `{ type, value }`. */
type OlText = string | { value?: string }

function textOf(t: OlText | undefined): string | undefined {
    return typeof t === 'string' ? t : t?.value
}

/** `api/books?jscmd=data` entry (the parts read here). */
interface OlBookData {
    title?: string
    authors?: { name?: string }[]
    languages?: { key?: string }[]
    notes?: OlText
    subjects?: { name?: string }[]
    subject_places?: { name?: string }[]
    identifiers?: { goodreads?: string[] }
}

interface OlSearchResponse {
    docs?: { key?: string, author_name?: string[] }[]
}

interface OlWork {
    title?: string
    subtitle?: unknown
    description?: OlText
    subjects?: unknown
    subject_places?: unknown
}

interface OlEdition {
    isbn_13?: string[]
    isbn_10?: string[]
    languages?: { key?: string }[]
}

interface OlEditionsResponse {
    entries?: OlEdition[]
}

function strings(value: unknown): string[] {
    return Array.isArray(value) ? value.filter((x): x is string => typeof x === 'string') : []
}

function names(list: { name?: string }[] | undefined): string[] {
    return (list ?? []).map(s => s.name).filter((n): n is string => typeof n === 'string')
}

function languageCode(key: string | undefined): string | undefined {
    return key?.replace('/languages/', '')
}

function categoriesFor(subjects: string[]): string[] | undefined {
    const category = detectCategory(subjects)
    return category ? [category] : undefined
}

function coverForIsbn(isbn: string): string {
    // ?default=false: a 404 instead of a grey placeholder image.
    return withoutPlaceholderCover(`https://covers.openlibrary.org/b/isbn/${encodeURIComponent(isbn)}-L.jpg`)
}

/**
 * Looks a book up by ISBN. Both forms (ISBN-13 and ISBN-10) are asked for in
 * one request, since Open Library files some editions under one form only.
 */
async function byIsbn(isbn: string): Promise<TidyupQueryResult | null> {
    const forms = isbnLookupForms(isbn)
    if (forms.length === 0) { return null }
    const bibkeys = forms.map(f => `ISBN:${f}`).join(',')
    const params = new URLSearchParams({ bibkeys, format: 'json', jscmd: 'data' })
    console.log('[tidyup:openlibrary] search by isbn', forms.join(', '))
    const data = await fetchJson<Record<string, OlBookData>>(`https://openlibrary.org/api/books?${params.toString()}`, { label: LABEL })

    const found = forms.find(f => data[`ISBN:${f}`])
    if (!found) { return null }
    const book = data[`ISBN:${found}`]

    const goodreadsId = book.identifiers?.goodreads?.[0]
    const descRaw = textOf(book.notes)
    return {
        metadata: stripUndefined({
            title:        book.title,
            author:       book.authors?.[0]?.name,
            language:     languageCode(book.languages?.[0]?.key),
            description:  descRaw ? stripHtml(descRaw) : undefined,
            // The book's own spelling: the same ISBN shouldn't show up as a change.
            isbn,
            categories:   categoriesFor([...names(book.subjects), ...names(book.subject_places)]),
            goodreadsUrl: goodreadsId ? `https://www.goodreads.com/book/show/${encodeURIComponent(goodreadsId)}` : undefined,
        }),
        coverUrl: coverForIsbn(found),
    }
}

/** Searches by title and author: the best work, then its details and editions. */
async function byTitle({ title, author, lang }: QueryParams): Promise<TidyupQueryResult | null> {
    if (!title && !author) {return null}

    // Step 1: resolve the work key with a search.
    const parts: string[] = []
    if (title)  {parts.push(`title:${title}`)}
    if (author) {parts.push(`author:${author}`)}
    if (lang)   {parts.push(`language:${lang}`)}
    const searchParams = new URLSearchParams({ q: parts.join(' '), limit: '1', fields: 'key,author_name' })
    const searchUrl = `https://openlibrary.org/search.json?${searchParams.toString()}`
    console.log('[tidyup:openlibrary] search', searchUrl)
    const searchData = await fetchJson<OlSearchResponse>(searchUrl, { label: LABEL })
    const doc = searchData.docs?.[0]
    // The key becomes part of the next URLs: only accept a work path, so a
    // strange answer can't send the requests elsewhere (`@host/…` would).
    if (!doc?.key?.startsWith('/works/')) {return null}

    const workKey = doc.key
    const authorName = doc.author_name?.[0]

    // Step 2: the work and its editions, in parallel. The editions only add
    // an ISBN and a language: without them the work is still a result.
    const [workData, editionsData] = await Promise.all([
        fetchJson<OlWork>(`https://openlibrary.org${workKey}.json`, { label: LABEL }),
        fetchJson<OlEditionsResponse>(`https://openlibrary.org${workKey}/editions.json?limit=50`, { label: LABEL })
            .catch((): OlEditionsResponse => ({ entries: [] })),
    ])

    const subjects = [...strings(workData.subjects), ...strings(workData.subject_places)]
    const descRaw = textOf(workData.description)

    const entries = editionsData.entries ?? []
    const hasIsbn = (e: OlEdition) => Boolean(e.isbn_13?.[0] || e.isbn_10?.[0])
    const langKey = lang ? `/languages/${lang}` : null
    const matchedEdition = (langKey
        ? entries.find(e => e.languages?.some(l => l.key === langKey) && hasIsbn(e))
        : undefined) ?? entries.find(hasIsbn)

    const foundIsbn = matchedEdition?.isbn_13?.[0] ?? matchedEdition?.isbn_10?.[0]
    const workId = workKey.replace('/works/', '')

    return {
        metadata: stripUndefined({
            title:       workData.title,
            subtitle:    typeof workData.subtitle === 'string' ? workData.subtitle : undefined,
            author:      authorName,
            language:    languageCode(matchedEdition?.languages?.[0]?.key),
            isbn:        foundIsbn,
            categories:  categoriesFor(subjects),
            description: descRaw ? stripHtml(descRaw) : undefined,
        }),
        coverUrl: foundIsbn
            ? coverForIsbn(foundIsbn)
            : withoutPlaceholderCover(`https://covers.openlibrary.org/b/olid/${encodeURIComponent(workId)}-L.jpg`),
    }
}

/** An ISBN is looked up by ISBN only; the caller falls back to a title search (see tidyupBook). */
function query(params: QueryParams): Promise<TidyupQueryResult | null> {
    return params.isbn ? byIsbn(params.isbn) : byTitle(params)
}

export const openLibraryProvider: TidyupProvider = {
    id: 'openlibrary',

    isAvailable(): boolean {
        return true
    },

    query,
}
