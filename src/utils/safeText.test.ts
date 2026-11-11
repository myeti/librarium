import { describe, expect, it } from 'vitest'
import { headingSlug, parseSafeMarkdown, parseSafeText, type MarkdownBlock } from './safeText'
import privacyPolicy from '../../PRIVACY.md?raw'

describe('parseSafeText', () => {
    it('splits paragraphs on blank lines and keeps single line breaks', () => {
        expect(parseSafeText('Hello\nworld\n\nSecond')).toEqual([
            [{ type: 'text', text: 'Hello' }, { type: 'br' }, { type: 'text', text: 'world' }],
            [{ type: 'text', text: 'Second' }],
        ])
    })

    it('turns http(s) URLs into links and trims trailing punctuation', () => {
        expect(parseSafeText('See https://librarium.app/news. Thanks')).toEqual([[
            { type: 'text', text: 'See ' },
            { type: 'link', href: 'https://librarium.app/news', text: 'https://librarium.app/news' },
            { type: 'text', text: '. Thanks' },
        ]])
    })

    it('keeps HTML as literal text (no tag survives as markup)', () => {
        const payload = '<img src=x onerror="alert(1)"><script>alert(2)</script>'
        const out = parseSafeText(payload)
        expect(out).toEqual([[{ type: 'text', text: payload }]])
    })

    it('never links javascript: or data: URLs', () => {
        const out = parseSafeText('javascript:alert(1) data:text/html,<b>x</b>')
        expect(out.flat().some(n => n.type === 'link')).toBe(false)
    })

    it('stops a URL at a quote, so it cannot break out of an attribute', () => {
        const out = parseSafeText('https://x.test/"onmouseover="alert(1)')
        const link = out[0].find(n => n.type === 'link')
        expect(link).toEqual({ type: 'link', href: 'https://x.test/', text: 'https://x.test/' })
    })

    it('reads legacy <br> and <p> tags as breaks', () => {
        expect(parseSafeText('<p>One<br/>Two</p><p>Three</p>')).toEqual([
            [{ type: 'text', text: 'One' }, { type: 'br' }, { type: 'text', text: 'Two' }],
            [{ type: 'text', text: 'Three' }],
        ])
    })

    it('returns nothing for empty input', () => {
        expect(parseSafeText('')).toEqual([])
        expect(parseSafeText(null)).toEqual([])
        expect(parseSafeText('  \n\n ')).toEqual([])
    })
})

describe('parseSafeMarkdown', () => {
    it('reads headings (with GitHub anchors), paragraphs and lists', () => {
        const md = [
            '# Title',
            '',
            'One line',
            'and the next.',
            '',
            "## What's new messages",
            '',
            '- first item',
            '  continued',
            '- second',
            '',
            '## Looking up metadata (Tidy up)',
        ].join('\n')
        expect(parseSafeMarkdown(md)).toEqual([
            { type: 'heading', level: 1, id: 'title', inlines: [{ type: 'text', text: 'Title' }] },
            { type: 'paragraph', inlines: [{ type: 'text', text: 'One line and the next.' }] },
            { type: 'heading', level: 2, id: 'whats-new-messages', inlines: [{ type: 'text', text: "What's new messages" }] },
            { type: 'list', items: [[{ type: 'text', text: 'first item continued' }], [{ type: 'text', text: 'second' }]] },
            { type: 'heading', level: 2, id: 'looking-up-metadata-tidy-up', inlines: [{ type: 'text', text: 'Looking up metadata (Tidy up)' }] },
        ] satisfies MarkdownBlock[])
    })

    it('reads emphasis, code, links and in-document anchors', () => {
        const [block] = parseSafeMarkdown('**Off** unless *you* say `yes`: see [Umami](https://umami.is), [below](#error-reports) or [the README](README.md).')
        expect(block).toEqual({
            type: 'paragraph',
            inlines: [
                { type: 'strong', text: 'Off' },
                { type: 'text', text: ' unless ' },
                { type: 'em', text: 'you' },
                { type: 'text', text: ' say ' },
                { type: 'code', text: 'yes' },
                { type: 'text', text: ': see ' },
                { type: 'link', href: 'https://umami.is/', text: 'Umami' },
                { type: 'text', text: ', ' },
                { type: 'anchor', target: 'error-reports', text: 'below' },
                { type: 'text', text: ' or the README.' },
            ],
        })
    })

    it('reads pipe tables, with pipes inside code kept in their cell', () => {
        const md = '| What | Sent to |\n|---|---|\n| Statistics | `a|b` and **c** |\n| Errors | x |'
        expect(parseSafeMarkdown(md)).toEqual([{
            type: 'table',
            header: [[{ type: 'text', text: 'What' }], [{ type: 'text', text: 'Sent to' }]],
            rows: [
                [[{ type: 'text', text: 'Statistics' }], [{ type: 'code', text: 'a|b' }, { type: 'text', text: ' and ' }, { type: 'strong', text: 'c' }]],
                [[{ type: 'text', text: 'Errors' }], [{ type: 'text', text: 'x' }]],
            ],
        }])
    })

    it('removes HTML comments, even across lines or unterminated', () => {
        const md = 'Before\n\n<!-- TODO(owner): secret\nnotes -->\n\nAfter <!-- aside --> text\n\nEnd <!-- never closed\nhidden'
        const text = JSON.stringify(parseSafeMarkdown(md))
        expect(text).not.toContain('TODO')
        expect(text).not.toContain('secret')
        expect(text).not.toContain('aside')
        expect(text).not.toContain('hidden')
        expect(text).toContain('Before')
        expect(text).toContain('After  text')
        expect(text).toContain('End')
    })

    it('keeps any other HTML as literal text, and never links javascript:', () => {
        const blocks = parseSafeMarkdown('<img src=x onerror="alert(1)"> [click](javascript:alert(1)) <script>alert(2)</script>')
        const inlines = blocks.flatMap(b => b.type === 'paragraph' ? b.inlines : [])
        expect(inlines.some(n => n.type === 'link')).toBe(false)
        expect(inlines.map(n => n.type === 'br' ? '' : n.text).join('')).toContain('<img src=x onerror="alert(1)">')
    })

    it('makes duplicate heading anchors unique', () => {
        const ids = parseSafeMarkdown('## Notes\n\n## Notes').map(b => b.type === 'heading' ? b.id : '')
        expect(ids).toEqual(['notes', 'notes-1'])
        expect(headingSlug('Builds you make yourself')).toBe('builds-you-make-yourself')
    })

    it('parses PRIVACY.md: every in-document link has its heading, and the owner’s TODO stays out', () => {
        const blocks = parseSafeMarkdown(privacyPolicy)
        const ids = new Set(blocks.flatMap(b => b.type === 'heading' ? [b.id] : []))
        const inlines = blocks.flatMap(b => {
            if (b.type === 'heading' || b.type === 'paragraph') { return b.inlines }
            if (b.type === 'list') { return b.items.flat() }
            return [...b.header.flat(), ...b.rows.flat(2)]
        })
        const anchors = inlines.filter(n => n.type === 'anchor')
        expect(anchors.length).toBeGreaterThan(5)
        for (const a of anchors) { expect(ids).toContain(a.target) }
        expect(privacyPolicy).toContain('TODO(owner)')
        expect(JSON.stringify(blocks)).not.toContain('TODO')
        expect(blocks.some(b => b.type === 'table')).toBe(true)
    })
})
