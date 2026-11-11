#!/usr/bin/env node
/**
 * Sets the app version everywhere it is stored.
 *
 * Usage:
 *   npm run bump 0.6.0              explicit version
 *   npm run bump -- --patch         0.6.0 → 0.6.1 (also --minor, --major)
 *   npm run bump -- 0.6.0 --commit  …and commit the changed files
 *
 * Writes package.json, package-lock.json, src-tauri/Cargo.toml ([package]
 * version) and src-tauri/Cargo.lock (the `librarium` entry), then prints what
 * changed. tauri.conf.json is not touched: it reads the version from
 * package.json. Nothing is committed unless you pass --commit.
 *
 * The lockfiles are edited directly (only the app's own version fields),
 * which is what `npm install --package-lock-only` and `cargo update -p
 * librarium` would do for a version-only change, but without network access
 * or re-resolving any dependency. CI builds with `npm ci` / `cargo --locked`,
 * so a lockfile that is out of step fails there.
 */
import { readFileSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { execFileSync } from 'node:child_process'
import {
    ROOT, FILES, SEMVER, compareSemver, findCargoPackageVersion,
    findCargoLockVersions, setCargoLockVersion, readVersions, versionProblems,
} from './versions.mjs'

function fail(message) {
    console.error(message)
    process.exit(1)
}

const args = process.argv.slice(2)
const commit = args.includes('--commit')
const explicit = args.filter(a => !a.startsWith('--'))
const step = ['--major', '--minor', '--patch'].filter(a => args.includes(a))
const unknown = args.filter(a => a.startsWith('--') && !['--commit', '--major', '--minor', '--patch'].includes(a))

if (unknown.length || explicit.length + step.length !== 1) {
    fail('Usage: npm run bump <X.Y.Z>   or   npm run bump -- --patch|--minor|--major [--commit]')
}

const path = rel => join(ROOT, rel)
const pkg = JSON.parse(readFileSync(path(FILES.packageJson), 'utf-8'))
const current = pkg.version
if (!SEMVER.test(current)) { fail(`package.json version "${current}" is not X.Y.Z; fix it by hand first.`) }

let next
if (explicit.length) {
    next = explicit[0].replace(/^v/, '')
} else {
    const [major, minor, patch] = current.split('.').map(Number)
    next = step[0] === '--major' ? `${major + 1}.0.0`
        : step[0] === '--minor' ? `${major}.${minor + 1}.0`
        : `${major}.${minor}.${patch + 1}`
}

if (!SEMVER.test(next)) {
    fail(`Invalid version "${next}": use X.Y.Z (no pre-release suffix; MSI installers reject it).`)
}
if (compareSemver(next, current) <= 0) {
    fail(`New version ${next} must be greater than the current ${current} (the updater only installs newer versions).`)
}

const before = readVersions()

// package.json
pkg.version = next
writeFileSync(path(FILES.packageJson), JSON.stringify(pkg, null, 2) + '\n')

// package-lock.json: root version fields only
const lock = JSON.parse(readFileSync(path(FILES.packageLock), 'utf-8'))
lock.version = next
if (lock.packages?.['']) { lock.packages[''].version = next }
writeFileSync(path(FILES.packageLock), JSON.stringify(lock, null, 2) + '\n')

// Cargo.toml: [package] version only
const cargoToml = readFileSync(path(FILES.cargoToml), 'utf-8')
const found = findCargoPackageVersion(cargoToml)
if (!found) { fail(`${FILES.cargoToml}: no version in [package]`) }
writeFileSync(path(FILES.cargoToml), cargoToml.slice(0, found.start) + next + cargoToml.slice(found.end))

// Cargo.lock: the librarium entry only
const cargoLock = readFileSync(path(FILES.cargoLock), 'utf-8')
if (findCargoLockVersions(cargoLock).length !== 1) { fail(`${FILES.cargoLock}: expected exactly one "librarium" entry`) }
writeFileSync(path(FILES.cargoLock), setCargoLockVersion(cargoLock, next))

const after = readVersions()
const rows = [
    [`${FILES.packageJson}`, before.packageJson, after.packageJson],
    [`${FILES.packageLock}`, before.packageLock, after.packageLock],
    [`${FILES.packageLock} (packages[""])`, before.packageLockRoot, after.packageLockRoot],
    [`${FILES.cargoToml}`, before.cargoToml, after.cargoToml],
    [`${FILES.cargoLock}`, before.cargoLock, after.cargoLock],
]
console.log(`Bumped ${current} → ${next}`)
for (const [file, from, to] of rows) {
    console.log(`  ${file.padEnd(36)} ${String(from).padEnd(10)} → ${to}`)
}
console.log(`  ${FILES.tauriConf.padEnd(36)} (reads ${after.tauriConf})`)

const problems = versionProblems(after)
if (problems.length) {
    console.error('\nSome copies still disagree:')
    for (const p of problems) { console.error(`  - ${p}`) }
    process.exit(1)
}

const changed = [FILES.packageJson, FILES.packageLock, FILES.cargoToml, FILES.cargoLock]
if (commit) {
    execFileSync('git', ['commit', '-m', `Bump ${next}`, '--', ...changed], { cwd: ROOT, stdio: 'inherit' })
} else {
    console.log(`\nNot committed. Next:\n  git commit -m "Bump ${next}" -- ${changed.join(' ')}\n  npm run release`)
}
