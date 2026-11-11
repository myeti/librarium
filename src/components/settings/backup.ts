import { t } from '@/i18n'
import { fmtSize } from '@/utils/format'
import type { BackupProgress } from '@/types'

/**
 * What a running backup or restore is doing, for people: "Listing the
 * files…", "312 of 1204 files · 1.2 GB of 4.5 GB", "Replacing the
 * library…". '' when nothing runs.
 */
export function backupProgressText(progress: BackupProgress | null): string {
    if (!progress) { return '' }
    switch (progress.phase) {
    case 'scanning':
        return t('settings.backup.scanning')
    case 'replacing':
        return t('settings.backup.replacing')
    case 'copying':
        return t('settings.backup.copying', {
            n: progress.filesTotal,
            done: progress.filesDone,
            bytes: fmtSize(progress.bytesDone),
            total: fmtSize(progress.bytesTotal),
        })
    }
}
