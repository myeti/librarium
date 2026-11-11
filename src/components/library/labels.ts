/** Accessible names for the library collection (pure, so they can be tested). */
import { t } from '@/i18n'
import { itemAuthor, itemSeries, itemTitle, type ShelfItem } from '@/services/shelf'

/**
 * What a screen reader announces for one item of the grid or list (the
 * option's name: an option's content isn't read out on its own):
 * "Dune, Frank Herbert, Dune #1, favorite, on Kindle, 42% read". Where the
 * book is is said only when it is on the device (`device`: its name).
 */
export function itemOptionLabel(item: ShelfItem, device: string): string {
    const parts = [itemTitle(item, t), itemAuthor(item, t)]
    const series = itemSeries(item)
    if (series) { parts.push(series) }
    if (item.book?.favorite) { parts.push(t('favorites.inLabel')) }
    if (item.presence === 'both') { parts.push(t('shelf.onDevice', { device })) }
    if (item.presence === 'deviceOnly') { parts.push(t('shelf.onlyOnDevice', { device })) }
    if (item.progress !== undefined) { parts.push(t('collection.progress', { percent: item.progress })) }
    return parts.join(', ')
}
