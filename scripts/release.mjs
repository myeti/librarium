#!/usr/bin/env node
/**
 * Tags the current version and pushes it, which starts the Release workflow
 * (.github/workflows/release.yml). Nothing is built or signed here: CI does
 * that. See DEPLOY.md.
 *
 * Usage:
 *   npm run release                         write the notes in your editor
 *   npm run release -- --notes-file FILE    take the notes from FILE
 *   npm run release -- --dry-run            run every check, change nothing
 *   npm run release -- --yes                don't ask before pushing
 *
 * Refuses to run unless:
 *   - package.json, package-lock.json, Cargo.toml and Cargo.lock agree, and
 *     tauri.conf.json reads its version from package.json;
 *   - you are on `main` with no uncommitted changes to tracked files;
 *   - `main` contains `origin/main` (after a fetch): it may be ahead (those
 *     commits are pushed with the tag), never behind or diverged;
 *   - the tag doesn't exist yet, locally or on origin (exact name match), and
 *     the version is higher than every existing v* tag.
 *
 * The release notes become the annotated tag's message; the workflow
 * publishes them on GitHub and in latest.json (the in-app update notes).
 * `main` and the tag are pushed atomically: both or neither.
 */
import { execFileSync, spawnSync } from 'node:child_process'
import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { createInterface } from 'node:readline/promises'
import { pathToFileURL } from 'node:url'
import { ROOT, SEMVER, compareSemver, readVersions, versionProblems } from './versions.mjs'

const BRANCH = 'main'
const REMOTE = 'origin'
const COMMENT = ';'

function git(args, opts = {}) {
    return execFileSync('git', args, { cwd: ROOT, encoding: 'utf-8', stdio: ['ignore', 'pipe', 'pipe'], ...opts }).trim()
}

function gitOk(args) {
    return spawnSync('git', args, { cwd: ROOT, stdio: 'ignore' }).status === 0
}

function fail(message) {
    console.error(`\n✗ ${message}`)
    process.exit(1)
}

/** Exact tag names from `git ls-remote --tags` output (peeled `^{}` lines folded). */
export function parseLsRemoteTags(output) {
    const names = new Set()
    for (const line of output.split('\n')) {
        const ref = line.split('\t')[1]
        if (ref?.startsWith('refs/tags/')) { names.add(ref.slice('refs/tags/'.length).replace(/\^\{\}$/, '')) }
    }
    return names
}

/** Highest X.Y.Z among `vX.Y.Z` tag names, or null. */
export function highestVersion(tagNames) {
    let best = null
    for (const name of tagNames) {
        const v = name.startsWith('v') ? name.slice(1) : null
        if (v && SEMVER.test(v) && (best === null || compareSemver(v, best) > 0)) { best = v }
    }
    return best
}

/** Notes template: one line per commit since the previous tag (bump commits left out). */
export function notesTemplate(version, subjects) {
    const lines = subjects.filter(s => !/^Bump \d+\.\d+\.\d+$/.test(s)).map(s => `- ${s}`)
    return [
        ...lines,
        '',
        `${COMMENT} Release notes for v${version}. They are shown on the GitHub release page`,
        `${COMMENT} and in the app's update prompt, so write them for users: turn the commit`,
        `${COMMENT} list above into something readable (Markdown is fine). Lines starting`,
        `${COMMENT} with "${COMMENT}" are dropped; an empty message aborts the release.`,
        '',
    ].join('\n')
}

function parseArgs(argv) {
    const opts = { dryRun: false, yes: false, notesFile: null }
    for (let i = 0; i < argv.length; i++) {
        const a = argv[i]
        if (a === '--dry-run') { opts.dryRun = true }
        else if (a === '--yes' || a === '-y') { opts.yes = true }
        else if (a === '--notes-file') { opts.notesFile = argv[++i] }
        else { fail(`Unknown argument "${a}". Usage: npm run release [-- --notes-file FILE] [--dry-run] [--yes]`) }
    }
    if (opts.notesFile !== null) {
        if (!opts.notesFile || !existsSync(opts.notesFile)) { fail(`--notes-file: file not found: ${opts.notesFile}`) }
        if (!readFileSync(opts.notesFile, 'utf-8').trim()) { fail(`--notes-file: ${opts.notesFile} is empty.`) }
    }
    return opts
}

async function confirm(question) {
    const rl = createInterface({ input: process.stdin, output: process.stdout })
    try {
        return /^y(es)?$/i.test((await rl.question(question)).trim())
    } finally {
        rl.close()
    }
}

async function main() {
    const opts = parseArgs(process.argv.slice(2))

    // 1. Versions
    const versions = readVersions()
    const problems = versionProblems(versions)
    if (problems.length) {
        fail(`Versions disagree:\n  - ${problems.join('\n  - ')}\nRun: npm run bump <X.Y.Z>, then commit.`)
    }
    const version = versions.packageJson
    const tag = `v${version}`
    console.log(`Releasing ${tag}`)

    // 2. Branch
    const branch = gitOk(['symbolic-ref', '-q', 'HEAD']) ? git(['symbolic-ref', '--short', 'HEAD']) : null
    if (branch !== BRANCH) { fail(`Releases are cut from "${BRANCH}", you are on ${branch ? `"${branch}"` : 'a detached HEAD'}.`) }

    // 3. Clean tree (tracked files); untracked files don't end up in the tag
    const dirty = git(['status', '--porcelain', '--untracked-files=no'])
    if (dirty) { fail(`Uncommitted changes:\n${dirty}\nCommit or stash them first.`) }
    const untracked = git(['status', '--porcelain', '--untracked-files=normal']).split('\n').filter(l => l.startsWith('??'))
    if (untracked.length) { console.warn(`! ${untracked.length} untracked file(s), not part of the release:\n${untracked.join('\n')}`) }

    // 4. In step with origin/main
    console.log(`Fetching ${REMOTE}/${BRANCH}…`)
    try {
        git(['fetch', '--quiet', REMOTE, `+refs/heads/${BRANCH}:refs/remotes/${REMOTE}/${BRANCH}`])
    } catch (e) {
        fail(`git fetch failed: ${e.stderr || e.message}`)
    }
    const head = git(['rev-parse', 'HEAD'])
    const upstream = git(['rev-parse', `${REMOTE}/${BRANCH}`])
    if (!gitOk(['merge-base', '--is-ancestor', upstream, head])) {
        fail(`${BRANCH} is behind or has diverged from ${REMOTE}/${BRANCH}. Pull/rebase first.`)
    }
    const ahead = Number(git(['rev-list', '--count', `${upstream}..${head}`]))

    // 5. Tag must be new, locally and on origin (exact names, not substrings)
    if (gitOk(['rev-parse', '-q', '--verify', `refs/tags/${tag}`])) {
        fail(`Tag ${tag} already exists locally. If it is a leftover, remove it with: git tag -d ${tag}`)
    }
    let remoteTags
    try {
        remoteTags = parseLsRemoteTags(git(['ls-remote', '--tags', REMOTE]))
    } catch (e) {
        fail(`git ls-remote failed: ${e.stderr || e.message}`)
    }
    if (remoteTags.has(tag)) { fail(`Tag ${tag} already exists on ${REMOTE}. Bump the version.`) }
    const localTags = git(['tag', '--list', 'v*']).split('\n').filter(Boolean)
    const highest = highestVersion([...remoteTags, ...localTags])
    if (highest && compareSemver(version, highest) <= 0) {
        fail(`${tag} is not higher than the latest tag v${highest}; the updater would ignore it.`)
    }

    console.log(`✓ versions agree (${version})`)
    console.log(`✓ on ${BRANCH}, clean, ${ahead ? `${ahead} commit(s) ahead of` : 'level with'} ${REMOTE}/${BRANCH}`)
    console.log(`✓ ${tag} is new${highest ? ` (latest is v${highest})` : ''}`)

    if (opts.dryRun) {
        console.log(`\nDry run: would create annotated tag ${tag} on ${head.slice(0, 12)} and push ${BRANCH} + ${tag} to ${REMOTE}.`)
        return
    }

    // 6. Annotated tag with the release notes
    let tagged
    if (opts.notesFile) {
        // "whitespace", not "strip": keeps Markdown headings such as "## Fixes"
        tagged = spawnSync('git', ['tag', '-a', tag, '-F', opts.notesFile, '--cleanup=whitespace'], { cwd: ROOT, stdio: 'inherit' })
    } else {
        const describe = ['describe', '--tags', '--abbrev=0', '--match', 'v[0-9]*']
        const previous = gitOk(describe) ? git(describe) : null
        const range = previous ? [`${previous}..HEAD`] : ['-n', '50', 'HEAD']
        const subjects = git(['log', '--no-merges', '--format=%s', ...range]).split('\n').filter(Boolean)
        const tmp = mkdtempSync(join(tmpdir(), 'librarium-release-'))
        try {
            const template = join(tmp, 'TAG_MESSAGE')
            writeFileSync(template, notesTemplate(version, subjects))
            // Template comments start with ";" so Markdown headings ("## Fixes") survive the strip.
            tagged = spawnSync('git', ['-c', `core.commentChar=${COMMENT}`, 'tag', '-a', tag, '-e', '-F', template, '--cleanup=strip'], { cwd: ROOT, stdio: 'inherit' })
        } finally {
            rmSync(tmp, { recursive: true, force: true })
        }
    }
    if (tagged.status !== 0) { fail('Tag not created (empty message or git error). Nothing was pushed.') }

    const message = git(['tag', '--list', '--format=%(contents)', tag])
    console.log(`\nCreated ${tag} on ${head.slice(0, 12)} with these notes:\n${'─'.repeat(60)}\n${message}\n${'─'.repeat(60)}`)

    // 7. Push main + tag atomically
    const what = `${ahead ? `${BRANCH} (${ahead} new commit(s)) and ` : ''}${tag}`
    if (!opts.yes && !(await confirm(`Push ${what} to ${REMOTE}? This starts the release. [y/N] `))) {
        git(['tag', '-d', tag])
        fail(`Not pushed; local tag ${tag} removed.`)
    }
    const push = spawnSync('git', ['push', '--atomic', REMOTE, `refs/heads/${BRANCH}:refs/heads/${BRANCH}`, `refs/tags/${tag}:refs/tags/${tag}`], { cwd: ROOT, stdio: 'inherit' })
    if (push.status !== 0) {
        git(['tag', '-d', tag])
        fail(`Push failed; local tag ${tag} removed so you can retry.`)
    }
    console.log(`\n✓ Pushed ${what}. Follow the Release workflow in the repository's Actions tab.`)
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
    await main()
}
