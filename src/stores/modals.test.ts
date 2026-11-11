import { beforeEach, describe, expect, it } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import { useModalsStore } from './modals'

beforeEach(() => { setActivePinia(createPinia()) })

describe('modals store', () => {
    it('stacks dialogs; the last opened is on top', () => {
        const modals = useModalsStore()
        expect(modals.hasOpenModal).toBe(false)
        expect(modals.topId).toBeNull()
        const editor = modals.open('MetadataModal')
        const confirm = modals.open('ConfirmDialog')
        expect(modals.count).toBe(2)
        expect(modals.isTop(confirm)).toBe(true)
        expect(modals.isTop(editor)).toBe(false)
        modals.close(confirm)
        expect(modals.isTop(editor)).toBe(true)
    })

    it('gives every dialog its own id', () => {
        const modals = useModalsStore()
        expect(modals.open('A')).not.toBe(modals.open('A'))
    })

    it('closes by id even out of order, and ignores unknown ids', () => {
        const modals = useModalsStore()
        const a = modals.open('A')
        const b = modals.open('B')
        modals.close(a)
        expect(modals.stack.map(e => e.id)).toEqual([b])
        modals.close('nope')
        modals.close(a)
        expect(modals.count).toBe(1)
    })
})
