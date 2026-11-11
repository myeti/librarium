/**
 * MTP Kindles (2024+, experimental; docs/IPC.md, "MTP Kindles").
 *
 * The backend gives an MTP Kindle the mount path `mtp://<port>` and its
 * books paths under it (`mtp://usb-20-1.2/documents/Dune.azw3`). These are
 * locators the backend understands, not files on this computer: they are
 * passed back to device commands as they are, but never shown as paths nor
 * given to the file manager ("Show in Finder").
 */
import type { ConnectedDevice } from '@/types'

const MTP_SCHEME = 'mtp://'

/** Whether `path` is an `mtp://` locator (an MTP Kindle or a book on one), not a file system path. */
export function isMtpPath(path: string): boolean {
    return path.startsWith(MTP_SCHEME)
}

/** Whether the device is an MTP Kindle (by its connection, or its `mtp://` mount path). */
export function isMtpDevice(device: Pick<ConnectedDevice, 'connection' | 'mountPath'>): boolean {
    return device.connection === 'mtp' || isMtpPath(device.mountPath)
}

/**
 * Where an MTP Kindle is plugged in, for a label that must tell two
 * identical Kindles apart: the USB port of its locator (`usb-20-1.2`).
 */
export function mtpPort(mountPath: string): string {
    return isMtpPath(mountPath) ? mountPath.slice(MTP_SCHEME.length).split('/')[0] : mountPath
}
