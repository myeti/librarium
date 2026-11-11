#!/usr/bin/env node
/**
 * Release workflow helpers (Node built-ins only, no npm dependency runs).
 *
 *   manifest  writes latest.json (updater manifest), SHA256SUMS and the
 *             GitHub release body for a flat directory of release assets
 *             whose updater signatures sit next to them as <file>.sig.
 *   verify    checks such a directory before publishing: latest.json parses,
 *             has a plain X.Y.Z version equal to --version and every
 *             platform entry; each entry's file exists and its signature
 *             verifies against the pubkey in tauri.conf.json (and is bound
 *             to the version); SHA256SUMS lists and matches every file.
 *
 *   node scripts/ci/release-assets.mjs manifest --dir DIR --version X.Y.Z \
 *        --notes NOTES.md --base-url URL --template TEMPLATE.md --body-out BODY.md
 *   node scripts/ci/release-assets.mjs verify --dir DIR --version X.Y.Z [--base-url URL]
 *
 * Updater keys: the plugin looks up "{os}-{arch}-{bundle}" first, then
 * "{os}-{arch}" (tauri-plugin-updater ≥ 2.10).
 *   - darwin-aarch64 / darwin-x86_64: the .app.tar.gz of each architecture.
 *   - windows-x86_64-nsis / -msi: each installer updates itself in place;
 *     plain windows-x86_64 (older clients, unknown bundle) gets NSIS, as before.
 *   - linux-x86_64-appimage only. There is deliberately no plain
 *     linux-x86_64: a .deb install would fall back to it and try to install
 *     an AppImage. .deb users update with the next .deb instead.
 */
import { createHash } from 'node:crypto'
import { readFileSync, readdirSync, statSync, writeFileSync } from 'node:fs'
import { basename, join } from 'node:path'
import { pathToFileURL } from 'node:url'
import { updaterPublicKey, verifyUpdaterArtifact } from './minisign.mjs'

const SEMVER = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/
const SUMS = 'SHA256SUMS'
const MANIFEST = 'latest.json'
export const UDEV_RULES = '70-librarium-kindle.rules'

const esc = s => s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')

/** File-name patterns of every asset a release must contain. */
export function assetPatterns(version) {
    const v = esc(version)
    return {
        macArmUpdater:   new RegExp(`_${v}_aarch64\\.app\\.tar\\.gz$`),
        macIntelUpdater: new RegExp(`_${v}_x64\\.app\\.tar\\.gz$`),
        macArmDmg:       new RegExp(`_${v}_aarch64\\.dmg$`),
        macIntelDmg:     new RegExp(`_${v}_x64\\.dmg$`),
        nsis:            new RegExp(`_${v}_x64-setup\\.exe$`),
        msi:             new RegExp(`_${v}_x64(_[A-Za-z-]+)?\\.msi$`),
        appimage:        new RegExp(`_${v}_amd64\\.AppImage$`),
        deb:             new RegExp(`_${v}_amd64\\.deb$`),
        udevRules:       new RegExp(`^${esc(UDEV_RULES)}$`),
    }
}

/** Maps each pattern to the single file that matches it; throws otherwise. */
export function classify(files, version) {
    const found = {}
    const problems = []
    for (const [kind, re] of Object.entries(assetPatterns(version))) {
        const hits = files.filter(f => re.test(f))
        if (hits.length === 1) { found[kind] = hits[0] }
        else { problems.push(`${kind}: expected one file matching ${re}, found ${hits.length ? hits.join(', ') : 'none'}`) }
    }
    if (problems.length) { throw new Error(`Release assets are incomplete:\n  - ${problems.join('\n  - ')}`) }
    return found
}

/** Updater platform key → asset kind. */
export const PLATFORMS = {
    'darwin-aarch64':        'macArmUpdater',
    'darwin-x86_64':         'macIntelUpdater',
    'windows-x86_64':        'nsis',
    'windows-x86_64-nsis':   'nsis',
    'windows-x86_64-msi':    'msi',
    'linux-x86_64-appimage': 'appimage',
}

/** Extra, non-updater field kept from the previous latest.json (direct installer links). */
const DOWNLOADS = {
    'darwin-aarch64':        'macArmDmg',
    'darwin-x86_64':         'macIntelDmg',
    'windows-x86_64-exe':    'nsis',
    'windows-x86_64-msi':    'msi',
    'linux-x86_64-appimage': 'appimage',
    'linux-x86_64-deb':      'deb',
}

function listFiles(dir) {
    return readdirSync(dir).filter(f => statSync(join(dir, f)).isFile()).sort()
}

const sha256 = file => createHash('sha256').update(readFileSync(file)).digest('hex')

export function buildManifest({ version, notes, pubDate, baseUrl, assets, readSig }) {
    const url = f => `${baseUrl.replace(/\/+$/, '')}/${encodeURIComponent(f)}`
    const platforms = {}
    for (const [key, kind] of Object.entries(PLATFORMS)) {
        platforms[key] = { signature: readSig(assets[kind]), url: url(assets[kind]) }
    }
    const downloads = {}
    for (const [key, kind] of Object.entries(DOWNLOADS)) { downloads[key] = url(assets[kind]) }
    return { version, notes, pub_date: pubDate, platforms, downloads }
}

function downloadsTable(assets, baseUrl) {
    const link = f => `[\`${f}\`](${baseUrl.replace(/\/+$/, '')}/${encodeURIComponent(f)})`
    return [
        '| Platform | Download |',
        '|---|---|',
        `| macOS, Apple Silicon (M1 or later) | ${link(assets.macArmDmg)} |`,
        `| macOS, Intel | ${link(assets.macIntelDmg)} |`,
        `| Windows 10/11, 64-bit | ${link(assets.nsis)} (recommended) or ${link(assets.msi)} |`,
        `| Linux x86_64, any distribution | ${link(assets.appimage)} (updates itself) |`,
        `| Linux x86_64, Debian/Ubuntu | ${link(assets.deb)} (update by installing the next .deb) |`,
    ].join('\n')
}

export function renderBody(template, { version, notes, assets, baseUrl }) {
    const values = {
        VERSION:   version,
        NOTES:     notes.trim(),
        DOWNLOADS: downloadsTable(assets, baseUrl),
        APPIMAGE:  assets.appimage,
        DEB:       assets.deb,
        RULES:     assets.udevRules,
    }
    return template.replace(/\{\{([A-Z]+)\}\}/g, (m, k) => {
        if (!(k in values)) { throw new Error(`template: unknown placeholder ${m}`) }
        return values[k]
    })
}

export function writeSums(dir) {
    const lines = listFiles(dir).filter(f => f !== SUMS).map(f => `${sha256(join(dir, f))}  ${f}`)
    writeFileSync(join(dir, SUMS), lines.join('\n') + '\n')
}

/** Returns a list of problems with the release in `dir` (empty = good). */
export function verifyRelease({ dir, version, baseUrl, pubkey }) {
    const problems = []
    const files = listFiles(dir)
    let manifest
    try {
        manifest = JSON.parse(readFileSync(join(dir, MANIFEST), 'utf-8'))
    } catch (e) {
        return [`${MANIFEST}: ${e.message}`]
    }
    if (typeof manifest.version !== 'string' || !SEMVER.test(manifest.version)) {
        problems.push(`${MANIFEST}: version ${JSON.stringify(manifest.version)} is not X.Y.Z`)
    } else if (manifest.version !== version) {
        problems.push(`${MANIFEST}: version ${manifest.version}, expected ${version}`)
    }
    if (typeof manifest.notes !== 'string' || !manifest.notes.trim()) { problems.push(`${MANIFEST}: empty notes`) }
    if (Number.isNaN(Date.parse(manifest.pub_date))) { problems.push(`${MANIFEST}: invalid pub_date`) }

    let assets = null
    try { assets = classify(files, version) } catch (e) { problems.push(e.message) }

    for (const [key, kind] of Object.entries(PLATFORMS)) {
        const entry = manifest.platforms?.[key]
        if (!entry) { problems.push(`${MANIFEST}: no "${key}" platform`); continue }
        let name
        try {
            const u = new URL(entry.url)
            if (u.protocol !== 'https:') { problems.push(`${key}: URL is not https`) }
            name = decodeURIComponent(basename(u.pathname))
        } catch {
            problems.push(`${key}: invalid URL ${entry.url}`)
            continue
        }
        if (baseUrl && !entry.url.startsWith(`${baseUrl.replace(/\/+$/, '')}/`)) {
            problems.push(`${key}: URL ${entry.url} is outside ${baseUrl}`)
        }
        if (assets && name !== assets[kind]) { problems.push(`${key}: points to ${name}, expected ${assets[kind]}`) }
        if (!files.includes(name)) { problems.push(`${key}: ${name} is not in the release`); continue }
        try {
            verifyUpdaterArtifact(join(dir, name), entry.signature, version, pubkey)
        } catch (e) {
            problems.push(`${key}: ${name}: ${e.message}`)
        }
    }
    for (const key of Object.keys(manifest.platforms ?? {})) {
        if (!(key in PLATFORMS)) { problems.push(`${MANIFEST}: unexpected platform "${key}"`) }
    }

    if (!files.includes(SUMS)) {
        problems.push(`${SUMS} is missing`)
    } else {
        const listed = new Map()
        for (const line of readFileSync(join(dir, SUMS), 'utf-8').split('\n').filter(Boolean)) {
            const m = line.match(/^([0-9a-f]{64}) [ *](.+)$/)
            if (!m) { problems.push(`${SUMS}: malformed line "${line}"`); continue }
            listed.set(m[2], m[1])
        }
        for (const f of files.filter(f => f !== SUMS)) {
            if (!listed.has(f)) { problems.push(`${SUMS}: ${f} is not listed`) }
            else if (listed.get(f) !== sha256(join(dir, f))) { problems.push(`${SUMS}: checksum mismatch for ${f}`) }
        }
        for (const f of listed.keys()) {
            if (!files.includes(f)) { problems.push(`${SUMS}: lists ${f}, which is not in the release`) }
        }
    }
    return problems
}

function parseArgs(argv) {
    const [command, ...rest] = argv
    const opts = {}
    for (let i = 0; i < rest.length; i += 2) {
        if (!rest[i].startsWith('--') || rest[i + 1] === undefined) { throw new Error(`bad argument ${rest[i]}`) }
        opts[rest[i].slice(2)] = rest[i + 1]
    }
    return { command, opts }
}

function need(opts, ...keys) {
    const missing = keys.filter(k => !opts[k])
    if (missing.length) { throw new Error(`missing --${missing.join(', --')}`) }
}

function main(argv) {
    const { command, opts } = parseArgs(argv)
    if (command === 'manifest') {
        need(opts, 'dir', 'version', 'notes', 'base-url', 'template', 'body-out')
        const files = listFiles(opts.dir).filter(f => f !== MANIFEST && f !== SUMS && !f.endsWith('.sig'))
        const assets = classify(files, opts.version)
        const notes = readFileSync(opts.notes, 'utf-8').trim()
        if (!notes) { throw new Error(`${opts.notes} is empty`) }
        const manifest = buildManifest({
            version: opts.version,
            notes,
            pubDate: new Date().toISOString().replace(/\.\d{3}Z$/, 'Z'),
            baseUrl: opts['base-url'],
            assets,
            readSig: f => readFileSync(join(opts.dir, `${f}.sig`), 'utf-8').trim(),
        })
        writeFileSync(join(opts.dir, MANIFEST), JSON.stringify(manifest, null, 2) + '\n')
        writeSums(opts.dir)
        const body = renderBody(readFileSync(opts.template, 'utf-8'), {
            version: opts.version, notes, assets, baseUrl: opts['base-url'],
        })
        writeFileSync(opts['body-out'], body)
        console.log(`Wrote ${MANIFEST}, ${SUMS} and ${opts['body-out']} for ${Object.keys(manifest.platforms).join(', ')}`)
    } else if (command === 'verify') {
        need(opts, 'dir', 'version')
        const problems = verifyRelease({ dir: opts.dir, version: opts.version, baseUrl: opts['base-url'], pubkey: updaterPublicKey() })
        if (problems.length) {
            console.error(`Release in ${opts.dir} is NOT publishable:\n  - ${problems.join('\n  - ')}`)
            process.exit(1)
        }
        console.log(`Release in ${opts.dir} verified: ${MANIFEST} ${opts.version}, ${Object.keys(PLATFORMS).length} signed platform entries, ${SUMS} complete.`)
    } else {
        throw new Error('usage: release-assets.mjs manifest|verify --dir DIR --version X.Y.Z …')
    }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
    try {
        main(process.argv.slice(2))
    } catch (e) {
        console.error(e.message)
        process.exit(1)
    }
}
