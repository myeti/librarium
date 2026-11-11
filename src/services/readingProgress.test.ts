import { describe, expect, it } from 'vitest'
import { deviceBookProgress } from './readingProgress'
import type { DeviceBook } from '@/types'

function onDevice(filename: string, extra: Partial<DeviceBook> = {}): DeviceBook {
    return { path: `/Volumes/Kindle/documents/${filename}`, filename, title: '', author: '', ...extra }
}

describe('deviceBookProgress', () => {
    it('prefers what a Kobo recorded, "finished" counting as 100', () => {
        expect(deviceBookProgress(onDevice('a.kepub', { readStatus: 'finished', percentRead: 20 }), {})).toBe(100)
        expect(deviceBookProgress(onDevice('a.kepub', { percentRead: 37.6 }), { a: 5 })).toBe(38)
    })

    it('else reads the progress map by file name without its extension', () => {
        expect(deviceBookProgress(onDevice('Dune.azw3'), { Dune: 42 })).toBe(42)
        expect(deviceBookProgress(onDevice('Dune.azw3'), {})).toBeUndefined()
        expect(deviceBookProgress(onDevice('Dune.azw3'), { Dune: 140 })).toBe(100)
    })
})
