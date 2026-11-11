#!/usr/bin/env node
/**
 * Verifies Tauri updater signatures (minisign, Ed25519) with Node's built-in
 * crypto only: no npm dependency runs here. This is independent from the
 * Tauri CLI that produced the signatures, so it catches a wrong key, a
 * truncated upload or a mismatched file before anything is published.
 *
 * Formats (all base64-wrapped by Tauri):
 *   public key: "untrusted comment: …\n" + b64("Ed" | key id (8) | key (32))
 *   signature:  "untrusted comment: …\n" + b64(alg (2) | key id (8) | sig (64)) + "\n"
 *               "trusted comment: …\n"   + b64(global sig (64))
 *   alg "ED" signs BLAKE2b-512(file), legacy "Ed" signs the file itself; the
 *   global signature covers sig (64) | trusted comment.
 *
 * Usage (release workflow, sign job):
 *   node scripts/ci/minisign.mjs --version X.Y.Z --sig-dir DIR FILE...
 *     checks DIR/<basename>.sig for every FILE, against the pubkey in
 *     src-tauri/tauri.conf.json, and that the signature is bound to X.Y.Z.
 */
import { createHash, createPublicKey, verify } from 'node:crypto'
import { readFileSync } from 'node:fs'
import { basename, join } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'

const ROOT = fileURLToPath(new URL('../..', import.meta.url))
// DER prefix of an Ed25519 SubjectPublicKeyInfo; the raw 32-byte key follows.
const ED25519_SPKI_PREFIX = Buffer.from('302a300506032b6570032100', 'hex')

function decodeB64Text(b64, what) {
    const text = Buffer.from(b64.trim(), 'base64').toString('utf-8')
    if (!text.startsWith('untrusted comment:')) { throw new Error(`${what}: not a base64-encoded minisign file`) }
    return text.split(/\r?\n/)
}

export function parsePublicKey(b64) {
    const lines = decodeB64Text(b64, 'public key')
    const raw = Buffer.from(lines[1] ?? '', 'base64')
    if (raw.length !== 42 || raw.subarray(0, 2).toString('latin1') !== 'Ed') {
        throw new Error('public key: unexpected format')
    }
    return {
        keyId: raw.subarray(2, 10),
        key: createPublicKey({ key: Buffer.concat([ED25519_SPKI_PREFIX, raw.subarray(10)]), format: 'der', type: 'spki' }),
    }
}

export function parseSignature(b64) {
    const lines = decodeB64Text(b64, 'signature')
    const raw = Buffer.from(lines[1] ?? '', 'base64')
    const trustedLine = lines[2] ?? ''
    const globalSig = Buffer.from(lines[3] ?? '', 'base64')
    const alg = raw.subarray(0, 2).toString('latin1')
    if (raw.length !== 74 || !['ED', 'Ed'].includes(alg)) { throw new Error('signature: unexpected format') }
    if (!trustedLine.startsWith('trusted comment: ') || globalSig.length !== 64) {
        throw new Error('signature: missing trusted comment')
    }
    return {
        prehashed: alg === 'ED',
        keyId: raw.subarray(2, 10),
        sig: raw.subarray(10),
        trustedComment: trustedLine.slice('trusted comment: '.length),
        globalSig,
    }
}

/**
 * Throws unless `signatureB64` is a valid signature of `data` by `publicKeyB64`.
 * Returns the (verified) trusted comment.
 */
export function verifyMinisign(data, signatureB64, publicKeyB64) {
    const pk = parsePublicKey(publicKeyB64)
    const s = parseSignature(signatureB64)
    if (!pk.keyId.equals(s.keyId)) {
        throw new Error(`signed with key ${Buffer.from(s.keyId).reverse().toString('hex').toUpperCase()}, `
            + `expected ${Buffer.from(pk.keyId).reverse().toString('hex').toUpperCase()}`)
    }
    const message = s.prehashed ? createHash('blake2b512').update(data).digest() : data
    if (!verify(null, message, pk.key, s.sig)) { throw new Error('signature does not match the file') }
    const global = Buffer.concat([s.sig, Buffer.from(s.trustedComment, 'utf-8')])
    if (!verify(null, global, pk.key, s.globalSig)) { throw new Error('trusted comment signature is invalid') }
    return s.trustedComment
}

/** The `version:` field Tauri writes into the trusted comment, if any. */
export function signedVersion(trustedComment) {
    return trustedComment.split('\t').find(f => f.startsWith('version:'))?.slice('version:'.length) ?? null
}

export function updaterPublicKey(root = ROOT) {
    const conf = JSON.parse(readFileSync(join(root, 'src-tauri/tauri.conf.json'), 'utf-8'))
    const key = conf?.plugins?.updater?.pubkey
    if (!key) { throw new Error('src-tauri/tauri.conf.json has no plugins.updater.pubkey') }
    return key
}

/** Verifies one file + signature and that the signature is bound to `version`. */
export function verifyUpdaterArtifact(file, signatureB64, version, publicKeyB64) {
    const comment = verifyMinisign(readFileSync(file), signatureB64, publicKeyB64)
    const signed = signedVersion(comment)
    if (signed !== version) {
        throw new Error(`signature is bound to version "${signed}", expected "${version}"`)
    }
}

function main(argv) {
    let version = null
    let sigDir = null
    const files = []
    for (let i = 0; i < argv.length; i++) {
        if (argv[i] === '--version') { version = argv[++i] }
        else if (argv[i] === '--sig-dir') { sigDir = argv[++i] }
        else { files.push(argv[i]) }
    }
    if (!version || !sigDir || files.length === 0) {
        console.error('Usage: node scripts/ci/minisign.mjs --version X.Y.Z --sig-dir DIR FILE...')
        process.exit(2)
    }
    const pubkey = updaterPublicKey()
    let failed = 0
    for (const file of files) {
        const sigPath = join(sigDir, `${basename(file)}.sig`)
        try {
            verifyUpdaterArtifact(file, readFileSync(sigPath, 'utf-8'), version, pubkey)
            console.log(`ok    ${basename(file)}`)
        } catch (e) {
            failed++
            console.error(`FAIL  ${basename(file)}: ${e.message}`)
        }
    }
    if (failed) { process.exit(1) }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
    main(process.argv.slice(2))
}
