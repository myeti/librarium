/**
 * Safe rendering of text that comes from the network (the "What's new"
 * messages served by the Librarium API).
 *
 * The webview is privileged: script running in it can call every Tauri
 * command, i.e. read and delete files. Server content must therefore never
 * reach `v-html`. Instead this module turns the text into a small tree of
 * paragraphs, line breaks, plain text and links, which the component renders
 * with ordinary Vue templates. Vue escapes text interpolation, so a payload
 * such as `<img src=x onerror=…>` is displayed as those literal characters
 * and never parsed as HTML.
 *
 * Supported formatting (the message format is plain text):
 * - a blank line starts a new paragraph, a single newline is a line break;
 * - `http://` and `https://` URLs become links that open in the browser;
 * - for messages written before this change, `<br>` and `<p>`/`</p>` tags are
 *   read as line/paragraph breaks. Every other tag is shown as text.
 */

export type SafeInline =
    | { type: 'text'; text: string }
    | { type: 'link'; href: string; text: string }
    | { type: 'br' }

export type SafeParagraph = SafeInline[]

// A URL runs until whitespace or a character that can't be part of one
// in running text. Trailing punctuation is trimmed afterwards.
const URL_RE = /https?:\/\/[^\s<>"']+/gi
const TRAILING_PUNCTUATION = /[.,;:!?)\]}»”’]+$/

function legacyTagsToNewlines(raw: string): string {
    return raw
        .replace(/\r\n?/g, '\n')
        .replace(/<br\s*\/?>/gi, '\n')
        .replace(/<\/p>\s*<p>/gi, '\n\n')
        .replace(/<\/?p>/gi, '\n\n')
}

/**
 * Only http(s) URLs that parse are turned into links. Anything else
 * (`javascript:`, `data:`, a malformed URL) stays plain text.
 */
function safeHref(candidate: string): string | null {
    try {
        const url = new URL(candidate)
        return url.protocol === 'http:' || url.protocol === 'https:' ? url.href : null
    } catch {
        return null
    }
}

function linkify(line: string): SafeInline[] {
    const out: SafeInline[] = []
    let last = 0
    for (const match of line.matchAll(URL_RE)) {
        let url = match[0]
        const trailing = url.match(TRAILING_PUNCTUATION)?.[0] ?? ''
        if (trailing) { url = url.slice(0, -trailing.length) }
        const start = match.index ?? 0
        const href = safeHref(url)
        if (!href) { continue }
        if (start > last) { out.push({ type: 'text', text: line.slice(last, start) }) }
        out.push({ type: 'link', href, text: url })
        last = start + url.length
    }
    if (last < line.length) { out.push({ type: 'text', text: line.slice(last) }) }
    return out
}

export function parseSafeText(raw: string | null | undefined): SafeParagraph[] {
    if (!raw) { return [] }
    const text = legacyTagsToNewlines(raw)
    return text
        .split(/\n\s*\n/)
        .map(p => p.trim())
        .filter(p => p.length > 0)
        .map(paragraph => {
            const inlines: SafeInline[] = []
            paragraph.split('\n').forEach((line, i) => {
                if (i > 0) { inlines.push({ type: 'br' }) }
                inlines.push(...linkify(line))
            })
            return inlines
        })
}

// ── Bundled Markdown (the privacy policy) ─────────────────────────────────

/** Inline content of a bundled Markdown document. */
export type MarkdownInline =
    | SafeInline
    | { type: 'strong'; text: string }
    | { type: 'em'; text: string }
    | { type: 'code'; text: string }
    /** A link to a heading of the same document (`[text](#slug)`). */
    | { type: 'anchor'; target: string; text: string }

export type MarkdownBlock =
    | { type: 'heading'; level: number; id: string; inlines: MarkdownInline[] }
    | { type: 'paragraph'; inlines: MarkdownInline[] }
    | { type: 'list'; items: MarkdownInline[][] }
    | { type: 'table'; header: MarkdownInline[][]; rows: MarkdownInline[][][] }

/**
 * `code`, **strong**, *em*, [text](target), and a backslash escape. Nothing
 * nests: the text inside is shown as is.
 */
const MD_INLINE_RE = /`([^`\n]+)`|\*\*([^*\n]+?)\*\*|\*([^*\s][^*\n]*?)\*|\[([^\]\n]+)\]\(([^)\s]+)\)|\\([\\`*_[\]()#|<>-])/g

function pushText(out: MarkdownInline[], text: string) {
    if (!text) { return }
    for (const node of linkify(text)) {
        const last = out[out.length - 1]
        if (node.type === 'text' && last?.type === 'text') {
            last.text += node.text
        } else {
            out.push(node)
        }
    }
}

function parseInlines(text: string): MarkdownInline[] {
    const out: MarkdownInline[] = []
    let last = 0
    for (const m of text.matchAll(MD_INLINE_RE)) {
        const start = m.index ?? 0
        pushText(out, text.slice(last, start))
        last = start + m[0].length
        const [, code, strong, em, linkText, target, escaped] = m
        if (code !== undefined) {
            out.push({ type: 'code', text: code })
        } else if (strong !== undefined) {
            out.push({ type: 'strong', text: strong })
        } else if (em !== undefined) {
            out.push({ type: 'em', text: em })
        } else if (linkText !== undefined) {
            if (target.startsWith('#')) {
                out.push({ type: 'anchor', target: target.slice(1), text: linkText })
            } else {
                const href = safeHref(target)
                // A link to a file of the repository (README.md…) means
                // nothing in the app: its text stays, as text.
                if (href) { out.push({ type: 'link', href, text: linkText }) } else { pushText(out, linkText) }
            }
        } else {
            pushText(out, escaped)
        }
    }
    pushText(out, text.slice(last))
    return out
}

/** GitHub's heading anchors: "What's new messages" → `whats-new-messages`. */
export function headingSlug(text: string): string {
    return text
        .toLowerCase()
        .replace(/[^\p{L}\p{N}\s_-]/gu, '')
        .trim()
        .replace(/\s/g, '-')
}

function inlineText(inlines: MarkdownInline[]): string {
    return inlines.map(n => n.type === 'br' ? ' ' : n.text).join('')
}

/** The cells of a table row; a `|` inside `code` or escaped as `\|` doesn't split. */
function tableCells(line: string): string[] {
    const body = line.trim().replace(/^\|/, '').replace(/\|$/, '')
    const cells: string[] = []
    let cell = ''
    let inCode = false
    for (let i = 0; i < body.length; i++) {
        const c = body[i]
        if (c === '\\' && body[i + 1] === '|') { cell += '\\|'; i++; continue }
        if (c === '`') { inCode = !inCode }
        if (c === '|' && !inCode) { cells.push(cell.trim()); cell = ''; continue }
        cell += c
    }
    cells.push(cell.trim())
    return cells
}

const HEADING_RE = /^(#{1,6})\s+(.*?)\s*#*$/
const LIST_ITEM_RE = /^[-*+]\s+(.*)$/
const TABLE_SEPARATOR_RE = /^\|?\s*:?-{3,}:?\s*(\|\s*:?-{3,}:?\s*)*\|?$/

/**
 * Parses a Markdown document bundled with the app (PRIVACY.md, imported at
 * build time) into blocks a component renders with ordinary templates —
 * never `v-html`, for the same reason as {@link parseSafeText}.
 *
 * Supported: `#` headings (with GitHub's anchors), paragraphs, `-` lists
 * (continuation lines indented), pipe tables, and the inline forms above;
 * bare http(s) URLs become links. HTML comments (`<!-- … -->`: notes for the
 * maintainer) are removed, an unterminated one up to the end; any other
 * HTML is shown as text.
 */
export function parseSafeMarkdown(raw: string | null | undefined): MarkdownBlock[] {
    if (!raw) { return [] }
    const text = raw
        .replace(/\r\n?/g, '\n')
        .replace(/<!--[\s\S]*?(?:-->|$)/g, '')
    const lines = text.split('\n')
    const blocks: MarkdownBlock[] = []
    const slugs = new Map<string, number>()
    let paragraph: string[] = []
    let list: string[] | null = null

    const flushParagraph = () => {
        if (paragraph.length) { blocks.push({ type: 'paragraph', inlines: parseInlines(paragraph.join(' ')) }) }
        paragraph = []
    }
    const flushList = () => {
        if (list?.length) { blocks.push({ type: 'list', items: list.map(parseInlines) }) }
        list = null
    }

    for (let i = 0; i < lines.length; i++) {
        const line = lines[i]
        const trimmed = line.trim()
        const indented = /^\s/.test(line)

        if (!trimmed) { flushParagraph(); flushList(); continue }

        const heading = indented ? null : HEADING_RE.exec(trimmed)
        if (heading) {
            flushParagraph(); flushList()
            const inlines = parseInlines(heading[2])
            const base = headingSlug(inlineText(inlines))
            const seen = slugs.get(base) ?? 0
            slugs.set(base, seen + 1)
            blocks.push({ type: 'heading', level: heading[1].length, id: seen ? `${base}-${seen}` : base, inlines })
            continue
        }

        if (trimmed.startsWith('|') && TABLE_SEPARATOR_RE.test(lines[i + 1]?.trim() ?? '')) {
            flushParagraph(); flushList()
            const header = tableCells(trimmed).map(parseInlines)
            const rows: MarkdownInline[][][] = []
            i += 2
            while (i < lines.length && lines[i].trim().startsWith('|')) {
                rows.push(tableCells(lines[i]).map(parseInlines))
                i++
            }
            i--
            blocks.push({ type: 'table', header, rows })
            continue
        }

        const item = indented ? null : LIST_ITEM_RE.exec(trimmed)
        if (item) {
            flushParagraph()
            list ??= []
            list.push(item[1])
            continue
        }
        if (list && indented) {
            list[list.length - 1] += ` ${trimmed}`
            continue
        }

        flushList()
        paragraph.push(trimmed)
    }
    flushParagraph()
    flushList()
    return blocks
}
