/**
 * Reading progress of books on the connected e-reader, for the progress
 * labels and bars of the library screen (services/shelf.ts gives each
 * shelf item its copies' progress).
 *
 * The device store holds what the device said (read-only here):
 * - `deviceBooks`: the books on the device; a Kobo adds its own
 *   `percentRead` / `readStatus` to each;
 * - `readingProgress`: progress (0–100) by device file name without its
 *   extension (Kindle `.sdr` data, the Kobo database).
 *
 * A library book is matched with its copy on the device the same way as
 * for "already on device" (services/deviceMatch.ts): by library id first
 * (the copies Librarium sends carry it), else, for device books without a
 * known id, by title and author or by file name.
 */
import type { DeviceBook } from '@/types'

/** A device book's progress, 0–100, or undefined when the device recorded none. */
export function deviceBookProgress(book: DeviceBook, progressByStem: Readonly<Record<string, number>>): number | undefined {
    if (book.readStatus === 'finished') { return 100 }
    if (book.percentRead !== undefined && book.percentRead > 0) { return clampPercent(book.percentRead) }
    const value = progressByStem[book.filename.replace(/\.[^.]+$/, '')]
    return typeof value === 'number' && Number.isFinite(value) ? clampPercent(value) : undefined
}

function clampPercent(value: number): number {
    return Math.round(Math.min(100, Math.max(0, value)))
}
