#!/usr/bin/env node
/**
 * Writes a throwaway npm project into DIR whose lockfile holds only the
 * @tauri-apps/cli entries of package-lock.json (same versions and integrity
 * hashes). `npm ci --ignore-scripts` in DIR then installs the Tauri CLI alone,
 * so the release workflow's signing job never installs or runs any other
 * dependency of the app.
 *
 *   node scripts/ci/tauri-cli-project.mjs DIR
 */
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'

const ROOT = fileURLToPath(new URL('../..', import.meta.url))
const dir = process.argv[2]
if (!dir) {
    console.error('Usage: node scripts/ci/tauri-cli-project.mjs DIR')
    process.exit(2)
}

const lock = JSON.parse(readFileSync(join(ROOT, 'package-lock.json'), 'utf-8'))
const cli = lock.packages['node_modules/@tauri-apps/cli']
if (!cli?.version || !cli.integrity) {
    console.error('package-lock.json has no locked @tauri-apps/cli')
    process.exit(1)
}

const name = 'librarium-tauri-signer'
const devDependencies = { '@tauri-apps/cli': cli.version }
const packages = { '': { name, devDependencies } }
for (const [key, entry] of Object.entries(lock.packages)) {
    if (key === 'node_modules/@tauri-apps/cli' || key.startsWith('node_modules/@tauri-apps/cli-')) {
        packages[key] = entry
    }
}

mkdirSync(dir, { recursive: true })
writeFileSync(join(dir, 'package.json'), JSON.stringify({ name, private: true, devDependencies }, null, 2) + '\n')
writeFileSync(join(dir, 'package-lock.json'), JSON.stringify({ name, lockfileVersion: 3, requires: true, packages }, null, 2) + '\n')
console.log(`@tauri-apps/cli ${cli.version}: ${Object.keys(packages).length - 1} lockfile entries written to ${dir}`)
