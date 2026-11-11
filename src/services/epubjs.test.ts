import { afterEach, describe, expect, it } from 'vitest'
// Both come with epub.js (its dependencies), so they are always installed.
import JSZip from 'jszip'
import xmldomPackage from '@xmldom/xmldom/package.json'
import ePub from 'epubjs'

/**
 * epub.js 0.3.93 (the reader's EPUB renderer) is unmaintained and depends on
 * `@xmldom/xmldom` ^0.7, whose releases up to 0.8.14 have high-severity
 * advisories. package.json `overrides` pins `@xmldom/xmldom` to a patched
 * 0.8.x (same API as 0.7). These tests prove epub.js still opens a book
 * with it:
 *
 * - with the DOM's own parser, as in the webview (epub.js only falls back to
 *   xmldom where `DOMParser` / `XMLSerializer` don't exist);
 * - with `DOMParser` and `XMLSerializer` removed, which forces every parse
 *   through the overridden xmldom.
 */

const OPF = `<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="2.0" unique-identifier="BookId">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:opf="http://www.idpf.org/2007/opf">
    <dc:title>Dune</dc:title>
    <dc:creator opf:role="aut">Frank Herbert</dc:creator>
    <dc:language>en</dc:language>
    <dc:identifier id="BookId">urn:uuid:librarium-test</dc:identifier>
  </metadata>
  <manifest>
    <item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
    <item id="ch2" href="ch2.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine toc="ncx">
    <itemref idref="ch1"/>
    <itemref idref="ch2"/>
  </spine>
</package>`

const NCX = `<?xml version="1.0" encoding="UTF-8"?>
<ncx xmlns="http://www.daisy.org/z3986/2005/ncx/" version="2005-1">
  <head><meta name="dtb:uid" content="urn:uuid:librarium-test"/></head>
  <docTitle><text>Dune</text></docTitle>
  <navMap>
    <navPoint id="np1" playOrder="1"><navLabel><text>Chapter 1</text></navLabel><content src="ch1.xhtml"/></navPoint>
    <navPoint id="np2" playOrder="2"><navLabel><text>Chapter 2</text></navLabel><content src="ch2.xhtml"/></navPoint>
  </navMap>
</ncx>`

const chapter = (n: number) => `<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Chapter ${n}</title></head>
<body><h1>Chapter ${n}</h1><p>A beginning is the time for taking the most delicate care.</p></body></html>`

/** A small, valid EPUB 2 as the reader receives it: an ArrayBuffer. */
async function fixtureEpub(): Promise<ArrayBuffer> {
    const zip = new JSZip()
    zip.file('mimetype', 'application/epub+zip', { compression: 'STORE' })
    zip.file('META-INF/container.xml', `<?xml version="1.0"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles>
</container>`)
    zip.file('OEBPS/content.opf', OPF)
    zip.file('OEBPS/toc.ncx', NCX)
    zip.file('OEBPS/ch1.xhtml', chapter(1))
    zip.file('OEBPS/ch2.xhtml', chapter(2))
    return zip.generateAsync({ type: 'arraybuffer' })
}

/** Opens the fixture with epub.js and reads what the reader relies on. */
async function openFixture() {
    const book = ePub(await fixtureEpub())
    await book.opened
    const metadata = await book.loaded.metadata
    const toc = await book.loaded.navigation
    const spine: string[] = []
    book.spine.each((item: { href: string }) => spine.push(item.href))
    const result = {
        title: metadata.title,
        creator: metadata.creator,
        language: metadata.language,
        spine,
        toc: toc.toc.map(entry => entry.label.trim()),
    }
    book.destroy()
    return result
}

const EXPECTED = {
    title: 'Dune',
    creator: 'Frank Herbert',
    language: 'en',
    spine: ['ch1.xhtml', 'ch2.xhtml'],
    toc: ['Chapter 1', 'Chapter 2'],
}

describe('epub.js with the patched @xmldom/xmldom', () => {
    const { DOMParser: domParser, XMLSerializer: xmlSerializer } = globalThis

    afterEach(() => {
        globalThis.DOMParser = domParser
        globalThis.XMLSerializer = xmlSerializer
    })

    it('is the patched release (0.8.15 or later), not the 0.7 epub.js asks for', () => {
        const [major, minor, patch] = xmldomPackage.version.split('.').map(Number)
        expect(major > 0 || minor > 8 || (minor === 8 && patch >= 15), xmldomPackage.version).toBe(true)
    })

    it('opens an EPUB from an ArrayBuffer with the DOM parser (as in the webview)', async () => {
        const opened = await openFixture()
        // Only the structure here: happy-dom's `getElementsByTagNameNS`
        // finds no `dc:` elements (a happy-dom bug: the elements do have the
        // Dublin Core namespace), so epub.js reads empty metadata in tests.
        // WebKit and WebView2 don't have that problem; the xmldom test below
        // checks the metadata.
        expect({ spine: opened.spine, toc: opened.toc }).toEqual({ spine: EXPECTED.spine, toc: EXPECTED.toc })
    })

    it('opens it through xmldom when the DOM has no XML parser', async () => {
        // epub.js checks `typeof DOMParser === 'undefined'` at each parse.
        Reflect.deleteProperty(globalThis, 'DOMParser')
        Reflect.deleteProperty(globalThis, 'XMLSerializer')
        expect(typeof DOMParser).toBe('undefined')
        expect(await openFixture()).toEqual(EXPECTED)
    })
})
