/**
 * The shelf: what the library screen shows. The library's books and the
 * books on the selected e-reader, merged into one list:
 *
 * - a library book that isn't on the device (`libraryOnly`);
 * - a library book with its copies on the device (`both`): one item, never
 *   shown twice;
 * - a file on the device that no library book matches (`deviceOnly`). It
 *   only has what the device listing says: a title, an author, a file name,
 *   a size and, sometimes, the reading progress. No cover, series, category,
 *   language or date added: filters and sorts on those leave it out or put
 *   it last.
 *
 * Device files are matched with library books exactly like the "already on
 * device" marks (services/deviceMatch.ts, `DeviceIndex`): by the library id
 * the copies Librarium sends carry, else (a file without an id from this
 * library) by the same title AND author, or the same file name. A file may
 * match several library books (an EPUB and a PDF of the same work): it is a
 * copy of each.
 *
 * Pure functions, so the store's getters stay small and this is tested on
 * its own.
 */
import type { Book, BookFormat, DeviceBook, FilterConfig, Presence, SortConfig } from '@/types'
import { bookMatchKey, fileStem } from '@/services/deviceMatch'
import { deviceBookProgress } from '@/services/readingProgress'
import { displayAuthor, displayTitle, seriesLabel, type Translate } from '@/utils/bookDisplay'
import { compareText } from '@/utils/collator'

export type { Presence }

export const PRESENCES: readonly Presence[] = ['libraryOnly', 'both', 'deviceOnly']

/** The quick switch above the books: everything, the library's books, the device's books. */
export type ShelfScope = 'all' | 'library' | 'device'

export const SHELF_SCOPES: readonly ShelfScope[] = ['all', 'library', 'device']

export function isShelfScope(value: unknown): value is ShelfScope {
    return typeof value === 'string' && (SHELF_SCOPES as readonly string[]).includes(value)
}

/** The id of a device-only item: this prefix and the file's path (never a library id, which is a UUID). */
export const DEVICE_ITEM_PREFIX = 'device:'

export interface ShelfItem {
    /** The collection's key: the library book's id, or `device:` + the file's path. */
    readonly id: string
    /** The library book; null for a file only on the device. */
    readonly book: Book | null
    /** Its files on the device: none for `libraryOnly`, the file itself for `deviceOnly`. */
    readonly copies: readonly DeviceBook[]
    readonly presence: Presence
    /** Reading progress on the device, 0–100, when the device recorded some. */
    readonly progress?: number
}

/** The id of the device-only item for a file. */
export function deviceItemId(file: DeviceBook): string {
    return DEVICE_ITEM_PREFIX + file.path
}

/** Whether `id` is a device-only item's (not a library book's). */
export function isDeviceItemId(id: string): boolean {
    return id.startsWith(DEVICE_ITEM_PREFIX)
}

/** The device file of a device-only item (`copies[0]`), else null. */
export function fileOf(item: ShelfItem): DeviceBook | null {
    return item.book ? null : item.copies[0] ?? null
}

const FORMAT_BY_EXTENSION: Readonly<Record<string, BookFormat>> = {
    epub: 'epub', kepub: 'epub', pdf: 'pdf', mobi: 'mobi', prc: 'mobi', azw: 'azw3', azw3: 'azw3', kfx: 'azw3', cbz: 'cbz',
}

/** A device file's format, from its extension (`Book.kepub.epub` is an EPUB). */
export function fileFormat(filename: string): BookFormat {
    const ext = filename.toLowerCase().split('.').pop() ?? ''
    return FORMAT_BY_EXTENSION[ext] ?? 'unknown'
}

/** The highest progress among a book's copies (a re-sent copy may have been read further). */
function progressOf(copies: readonly DeviceBook[], progressByStem: Readonly<Record<string, number>>): number | undefined {
    let best: number | undefined
    for (const copy of copies) {
        const p = deviceBookProgress(copy, progressByStem)
        if (p !== undefined && (best === undefined || p > best)) { best = p }
    }
    return best
}

function push<K, V>(map: Map<K, V[]>, key: K, value: V): void {
    const list = map.get(key)
    if (list) { list.push(value) } else { map.set(key, [value]) }
}

/**
 * The merged list, in library order then device order (unsorted: see
 * `visibleShelf`). Without device books it is the library's books, each
 * `libraryOnly`.
 *
 * @param progressByStem the device store's `readingProgress`.
 */
export function buildShelf(
    books: readonly Book[],
    deviceBooks: readonly DeviceBook[],
    progressByStem: Readonly<Record<string, number>>,
): ShelfItem[] {
    const byId = new Map<string, Book>()
    const byKey = new Map<string, Book[]>()
    const byStem = new Map<string, Book[]>()
    for (const book of books) {
        byId.set(book.id, book)
        const key = bookMatchKey(book.metadata.title, book.metadata.author)
        if (key) { push(byKey, key, book) }
        push(byStem, fileStem(book.filename), book)
    }

    const copiesOf = new Map<string, DeviceBook[]>()
    const deviceOnly: DeviceBook[] = []
    for (const file of deviceBooks) {
        // A copy Librarium sent from this library is that book and nothing else.
        const sent = file.libraryId ? byId.get(file.libraryId) : undefined
        if (sent) {
            push(copiesOf, sent.id, file)
            continue
        }
        const key = bookMatchKey(file.title, file.author)
        const matches = new Set([...(key ? byKey.get(key) ?? [] : []), ...(byStem.get(fileStem(file.filename)) ?? [])])
        if (matches.size === 0) {
            deviceOnly.push(file)
        } else {
            for (const book of matches) { push(copiesOf, book.id, file) }
        }
    }

    const items: ShelfItem[] = books.map(book => {
        const copies = copiesOf.get(book.id) ?? []
        if (copies.length === 0) { return { id: book.id, book, copies, presence: 'libraryOnly' } }
        return { id: book.id, book, copies, presence: 'both', progress: progressOf(copies, progressByStem) }
    })
    for (const file of deviceOnly) {
        items.push({
            id: deviceItemId(file),
            book: null,
            copies: [file],
            presence: 'deviceOnly',
            progress: progressOf([file], progressByStem),
        })
    }
    return items
}

// ── What a filter or a sort sees of an item ──────────────────────────────

/** An item's fields for searching, filtering and sorting: the book's, or what the device says. */
interface ItemFields {
    title: string
    author: string
    series: string
    seriesIndex: number
    format: BookFormat
    /** ISO 8601, '' for a device-only file (unknown: last). */
    dateAdded: string
    /** -1 when unknown (last). */
    sizeBytes: number
    language: string
    categories: readonly string[]
    search: string
}

/** Lower-cased, without accents: "elise" finds "Élise". */
export function foldForSearch(s: string): string {
    return s.normalize('NFD').replace(/\p{M}+/gu, '').toLowerCase()
}

/** Items are immutable: their fields are computed once (a WeakMap: they go with the item). */
const fieldsCache = new WeakMap<object, ItemFields>()

function fieldsOf(item: ShelfItem): ItemFields {
    const owner: object = item.book ?? item.copies[0]
    let fields = fieldsCache.get(owner)
    if (fields) { return fields }
    if (item.book) {
        const { metadata: m } = item.book
        fields = {
            title: m.title, author: m.author, series: m.series ?? '', seriesIndex: m.seriesIndex ?? 0,
            format: item.book.format, dateAdded: item.book.dateAdded, sizeBytes: item.book.sizeBytes,
            language: m.language ?? '', categories: m.categories ?? [],
            search: foldForSearch([m.title, m.author, m.series ?? ''].join('\n')),
        }
    } else {
        const file = item.copies[0]
        fields = {
            title: file.title, author: file.author, series: '', seriesIndex: 0,
            format: fileFormat(file.filename), dateAdded: '', sizeBytes: file.sizeBytes ?? -1,
            language: '', categories: [],
            search: foldForSearch([file.title, file.author, file.filename].join('\n')),
        }
    }
    fieldsCache.set(owner, fields)
    return fields
}

/** The title shown for an item (a device file without one: its file name, without the extension). */
export function itemTitle(item: ShelfItem, t: Translate): string {
    if (item.book) { return displayTitle(item.book, t) }
    const file = item.copies[0]
    return file.title.trim() || file.filename.replace(/\.[^.]+$/, '')
}

/** The author shown for an item ("Unknown author" when there is none). */
export function itemAuthor(item: ShelfItem, t: Translate): string {
    return displayAuthor(item.book ?? item.copies[0], t)
}

/** "Dune #2" for a library book in a series; null otherwise (device files have none). */
export function itemSeries(item: ShelfItem): string | null {
    return item.book ? seriesLabel(item.book) : null
}

/** The format badge of an item: the book's, or the device file's from its extension. */
export function itemFormat(item: ShelfItem): BookFormat {
    return item.book ? item.book.format : fileFormat(item.copies[0].filename)
}

/**
 * The items to show: in `scope`, matching the search (title, author or
 * series; a device file's name too; case and accents ignored) and the
 * filters, then sorted. A device-only file has no series, language or
 * category, so a filter on one of those leaves it out.
 */
export function visibleShelf(items: readonly ShelfItem[], scope: ShelfScope, filters: FilterConfig, sort: SortConfig): ShelfItem[] {
    const q = foldForSearch(filters.search.trim())
    const result = items.filter(item => {
        if (scope === 'library' && item.presence === 'deviceOnly') { return false }
        if (scope === 'device' && item.presence === 'libraryOnly') { return false }
        if (filters.presence.length > 0 && !filters.presence.includes(item.presence)) { return false }
        const f = fieldsOf(item)
        if (q && !f.search.includes(q)) { return false }
        if (filters.formats.length > 0 && !filters.formats.includes(f.format)) { return false }
        if (filters.authors.length > 0 && !filters.authors.includes(f.author)) { return false }
        if (filters.series.length > 0 && !(f.series && filters.series.includes(f.series))) { return false }
        if (filters.languages.length > 0 && !(f.language && filters.languages.includes(f.language))) { return false }
        if (filters.categories.length > 0 && !f.categories.some(c => filters.categories.includes(c))) { return false }
        return true
    })
    return result.sort(comparator(sort))
}

/**
 * Favorites (starred books) come first, whatever the sort, then the others;
 * each group in the sort's order. Text fields sort with the shared collator
 * ("Élise" before "Zola", "Tome 2" before "Tome 10"); unknown values (an
 * empty title, author or series, a device file's date added or size, no
 * progress) go last in both directions; books of one series are in volume
 * order.
 */
function comparator({ field, order }: SortConfig): (a: ShelfItem, b: ShelfItem) => number {
    const dir = order === 'asc' ? 1 : -1
    return (a, b) => {
        const starred = Number(b.book?.favorite === true) - Number(a.book?.favorite === true)
        if (starred !== 0) { return starred }
        const fa = fieldsOf(a)
        const fb = fieldsOf(b)
        const blank = blanksLast(unknownOf(a, fa, field), unknownOf(b, fb, field))
        if (blank !== null) { return blank }
        const cmp = compareFields(a, b, fa, fb, field)
        if (cmp === 0 && field === 'series') {
            // Volumes of one series always read in order.
            return fa.seriesIndex - fb.seriesIndex
        }
        return dir * cmp
    }
}

/** Whether the item has no value for `field` (it then goes last). */
function unknownOf(item: ShelfItem, f: ItemFields, field: SortConfig['field']): boolean {
    switch (field) {
    case 'title':     return !f.title.trim()
    case 'author':    return !f.author.trim()
    case 'series':    return !f.series.trim()
    case 'dateAdded': return !f.dateAdded
    case 'sizeBytes': return f.sizeBytes < 0
    case 'progress':  return item.progress === undefined
    default:          return false
    }
}

/** 1 / -1 when only one of the two is unknown (it goes last), null otherwise (compared as usual). */
function blanksLast(aUnknown: boolean, bUnknown: boolean): number | null {
    if (aUnknown === bUnknown) { return null }
    return aUnknown ? 1 : -1
}

/** Orders two items by `field`, ascending (both have a value). */
function compareFields(a: ShelfItem, b: ShelfItem, fa: ItemFields, fb: ItemFields, field: SortConfig['field']): number {
    switch (field) {
    case 'title':     return compareText(fa.title, fb.title)
    case 'author':    return compareText(fa.author, fb.author)
    case 'series':    return compareText(fa.series, fb.series)
    case 'format':    return compareText(fa.format, fb.format)
    case 'sizeBytes': return fa.sizeBytes - fb.sizeBytes
    case 'progress':  return (a.progress ?? 0) - (b.progress ?? 0) || compareText(fa.title, fb.title)
        // ISO 8601 strings: plain comparison is chronological.
    case 'dateAdded':
    default:          return fa.dateAdded < fb.dateAdded ? -1 : fa.dateAdded > fb.dateAdded ? 1 : 0
    }
}
