#!/usr/bin/env node
/**
 * Where Librarium's version lives, and a check that every copy agrees.
 *
 * `package.json` is the source of truth. `src-tauri/tauri.conf.json` points at
 * it (`"version": "../package.json"`), and these copies must match it:
 *   - `package-lock.json` (root `version` and `packages[""].version`)
 *   - `src-tauri/Cargo.toml` (`[package] version`)
 *   - `src-tauri/Cargo.lock` (the `librarium` entry)
 *
 * Usage:
 *   node scripts/versions.mjs               check that every copy agrees
 *   node scripts/versions.mjs --tag v1.2.3  ...and that the tag is `v<version>`
 *
 * Used by `bump.mjs`, `release.mjs`, CI and the release workflow.
 */
import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'

export const ROOT = fileURLToPath(new URL('..', import.meta.url))
export const TAURI_VERSION_REF = '../package.json'
/** Plain `X.Y.Z`: MSI installers and the updater both need a numeric version. */
export const SEMVER = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/

export const FILES = {
    packageJson: 'package.json',
    packageLock: 'package-lock.json',
    cargoToml:   'src-tauri/Cargo.toml',
    cargoLock:   'src-tauri/Cargo.lock',
    tauriConf:   'src-tauri/tauri.conf.json',
}

const CARGO_LOCK_ENTRY = /(\[\[package\]\]\r?\nname = "librarium"\r?\nversion = ")([^"]*)(")/g

export function compareSemver(a, b) {
    const pa = a.split('.').map(Number)
    const pb = b.split('.').map(Number)
    for (let i = 0; i < 3; i++) {
        if (pa[i] !== pb[i]) { return pa[i] < pb[i] ? -1 : 1 }
    }
    return 0
}

/**
 * Finds `version = "…"` inside the `[package]` table of a Cargo.toml.
 * Returns `{ value, start, end }` (offsets of the value) or null.
 */
export function findCargoPackageVersion(toml) {
    const lines = toml.split('\n')
    let offset = 0
    let inPackage = false
    for (const line of lines) {
        const header = line.match(/^\s*\[([^\]]+)\]\s*(#.*)?\r?$/)
        if (header) {
            inPackage = header[1].trim() === 'package'
        } else if (inPackage) {
            const m = line.match(/^(\s*version\s*=\s*")([^"]*)"/)
            if (m) {
                const start = offset + m[1].length
                return { value: m[2], start, end: start + m[2].length }
            }
        }
        offset += line.length + 1
    }
    return null
}

/** Every Cargo.lock `librarium` entry's version (there must be exactly one). */
export function findCargoLockVersions(lock) {
    return [...lock.matchAll(CARGO_LOCK_ENTRY)].map(m => m[2])
}

export function setCargoLockVersion(lock, version) {
    return lock.replace(CARGO_LOCK_ENTRY, (_, pre, _old, post) => `${pre}${version}${post}`)
}

export function readText(rel, root = ROOT) {
    return readFileSync(join(root, rel), 'utf-8')
}

export function readVersions(root = ROOT) {
    const pkg = JSON.parse(readText(FILES.packageJson, root))
    const lock = JSON.parse(readText(FILES.packageLock, root))
    const tauri = JSON.parse(readText(FILES.tauriConf, root))
    const cargoToml = findCargoPackageVersion(readText(FILES.cargoToml, root))
    const cargoLock = findCargoLockVersions(readText(FILES.cargoLock, root))
    return {
        packageJson:     pkg.version,
        packageLock:     lock.version,
        packageLockRoot: lock.packages?.['']?.version,
        cargoToml:       cargoToml?.value,
        cargoLock:       cargoLock.length === 1 ? cargoLock[0] : undefined,
        cargoLockCount:  cargoLock.length,
        tauriConf:       tauri.version,
    }
}

/** Returns a list of human-readable problems (empty when everything agrees). */
export function versionProblems(v, tag) {
    const problems = []
    const want = v.packageJson
    if (!want || !SEMVER.test(want)) {
        problems.push(`${FILES.packageJson}: version "${want}" is not X.Y.Z`)
    }
    const copies = [
        [`${FILES.packageLock} (version)`, v.packageLock],
        [`${FILES.packageLock} (packages[""].version)`, v.packageLockRoot],
        [`${FILES.cargoToml} ([package] version)`, v.cargoToml],
    ]
    for (const [where, got] of copies) {
        if (got !== want) { problems.push(`${where} is "${got}", expected "${want}"`) }
    }
    if (v.cargoLockCount !== 1) {
        problems.push(`${FILES.cargoLock}: expected one "librarium" entry, found ${v.cargoLockCount}`)
    } else if (v.cargoLock !== want) {
        problems.push(`${FILES.cargoLock} (librarium) is "${v.cargoLock}", expected "${want}"`)
    }
    if (v.tauriConf !== TAURI_VERSION_REF) {
        problems.push(`${FILES.tauriConf}: version must be "${TAURI_VERSION_REF}" (single source), found "${v.tauriConf}"`)
    }
    if (tag !== undefined && tag !== `v${want}`) {
        problems.push(`tag "${tag}" does not match package.json version (expected "v${want}")`)
    }
    return problems
}

function main(argv) {
    const tagIndex = argv.indexOf('--tag')
    const tag = tagIndex >= 0 ? argv[tagIndex + 1] : undefined
    if (tagIndex >= 0 && !tag) {
        console.error('Usage: node scripts/versions.mjs [--tag vX.Y.Z]')
        process.exit(2)
    }
    const versions = readVersions()
    const problems = versionProblems(versions, tag)
    if (problems.length) {
        console.error('Version mismatch:')
        for (const p of problems) { console.error(`  - ${p}`) }
        console.error('Fix with: npm run bump <X.Y.Z>')
        process.exit(1)
    }
    console.log(`All versions agree: ${versions.packageJson}${tag ? ` (tag ${tag})` : ''}`)
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
    main(process.argv.slice(2))
}
