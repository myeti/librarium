import { afterEach, describe, expect, it } from 'vitest'
import { focusableWithin, isTypingTarget, uniqueId } from './dom'

afterEach(() => { document.body.innerHTML = '' })

describe('focusableWithin', () => {
    it('lists keyboard-focusable elements in order, skipping disabled, tabindex=-1, inert and hidden ones', () => {
        document.body.innerHTML = `
            <div id="root">
                <button id="a">a</button>
                <button id="b" disabled>b</button>
                <a id="c" href="#">c</a>
                <a id="d">no href</a>
                <input id="e" />
                <input id="f" type="hidden" />
                <div id="g" tabindex="0">g</div>
                <div id="h" tabindex="-1">h</div>
                <div inert><button id="i">i</button></div>
                <div hidden><button id="j">j</button></div>
                <textarea id="k"></textarea>
            </div>`
        const ids = focusableWithin(document.getElementById('root')!).map(el => el.id)
        expect(ids).toEqual(['a', 'c', 'e', 'g', 'k'])
    })
})

describe('isTypingTarget', () => {
    it('is true for text fields, selects and editable content', () => {
        const text = document.createElement('input')
        const area = document.createElement('textarea')
        const select = document.createElement('select')
        const editable = document.createElement('div')
        editable.contentEditable = 'true'
        document.body.append(editable)
        expect(isTypingTarget(text)).toBe(true)
        expect(isTypingTarget(area)).toBe(true)
        expect(isTypingTarget(select)).toBe(true)
        expect(isTypingTarget(editable)).toBe(true)
    })

    it('is false for checkboxes, buttons and plain elements', () => {
        const box = document.createElement('input')
        box.type = 'checkbox'
        expect(isTypingTarget(box)).toBe(false)
        expect(isTypingTarget(document.createElement('button'))).toBe(false)
        expect(isTypingTarget(document.body)).toBe(false)
        expect(isTypingTarget(null)).toBe(false)
    })
})

describe('uniqueId', () => {
    it('never repeats', () => {
        const ids = new Set(Array.from({ length: 50 }, () => uniqueId('x')))
        expect(ids.size).toBe(50)
        expect([...ids][0]).toMatch(/^x-\d+$/)
    })
})
