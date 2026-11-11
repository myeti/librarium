import type { ConnectedDevice } from '@/types'
import { t } from '@/i18n'

/**
 * Short device name for buttons and tooltips ("Send to Kindle"). Falls back
 * to the translated generic word when nothing is connected.
 */
export function deviceLabel(device: ConnectedDevice | null | undefined): string {
    if (!device) { return t('nav.device') }
    return device.deviceType === 'kobo' ? 'Kobo' : 'Kindle'
}
