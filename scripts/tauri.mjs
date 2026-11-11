#!/usr/bin/env node
/**
 * Runs the Tauri CLI with this build's own servers allowed by the CSP.
 *
 * The CSP in `src-tauri/tauri.conf.json` lists only the hosts every build
 * talks to (the book metadata and catalog services). The Librarium API and
 * the analytics server are build settings, `VITE_API_URL` and
 * `VITE_UMAMI_URL` (`.env` files locally, secrets in CI; see `.env.example`):
 * their origins are added to `connect-src` in both `csp` and `devCsp`, and
 * the result is passed to the CLI with `--config` (merged into
 * tauri.conf.json for this run only). Unset, nothing is added, and the app
 * reaches no server of its own.
 *
 * Usage (`npm run dev` and `npm run build` go through it; CI too):
 *   node scripts/tauri.mjs dev
 *   node scripts/tauri.mjs build --ci --target aarch64-apple-darwin -- --locked
 *
 * The values are read as Vite reads them for the same command (`dev`:
 * development mode, anything else: production), so the CSP always matches
 * what the frontend was built with.
 */
import { spawnSync } from 'node:child_process'
import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'

/** The build settings naming servers the app talks to. */
export const SERVER_SETTINGS = ['VITE_API_URL', 'VITE_UMAMI_URL']

/**
 * The origins (`https://host[:port]`) of the servers set in `env`, once
 * each. A value that isn't an http(s) URL is refused: it would end up inside
 * the CSP.
 */
export function serverOrigins(env) {
    const origins = []
    for (const name of SERVER_SETTINGS) {
        const value = env[name]?.trim()
        if (!value) { continue }
        let url
        try { url = new URL(value) } catch { throw new Error(`${name} isn't a URL: ${value}`) }
        if (url.protocol !== 'https:' && url.protocol !== 'http:') {
            throw new Error(`${name} must be an http(s) URL: ${value}`)
        }
        if (!origins.includes(url.origin)) { origins.push(url.origin) }
    }
    return origins
}

/** `csp` with `origins` appended to its `connect-src` directive. */
export function withConnectOrigins(csp, origins) {
    if (origins.length === 0) { return csp }
    if (!/(^|;)\s*connect-src\b/.test(csp)) { throw new Error('the CSP has no connect-src directive') }
    return csp.replace(/((?:^|;)\s*connect-src\b[^;]*)/, `$1 ${origins.join(' ')}`)
}

/** The `--config` override for `origins`, from tauri.conf.json's policies (null when there are none). */
export function cspOverride(security, origins) {
    if (origins.length === 0) { return null }
    return {
        app: {
            security: {
                csp: withConnectOrigins(security.csp, origins),
                devCsp: withConnectOrigins(security.devCsp, origins),
            },
        },
    }
}

/** `args` with `--config <json>` before any `--` (what follows `--` goes to cargo). */
export function withConfigArg(args, override) {
    if (!override) { return args }
    const end = args.indexOf('--')
    const at = end === -1 ? args.length : end
    return [...args.slice(0, at), '--config', JSON.stringify(override), ...args.slice(at)]
}

async function main() {
    const root = fileURLToPath(new URL('..', import.meta.url))
    const args = process.argv.slice(2)
    const { loadEnv } = await import('vite')
    const mode = args[0] === 'dev' ? 'development' : 'production'
    const origins = serverOrigins(loadEnv(mode, root, 'VITE_'))
    const security = JSON.parse(readFileSync(join(root, 'src-tauri', 'tauri.conf.json'), 'utf-8')).app.security
    if (origins.length > 0) { console.log(`[tauri.mjs] CSP connect-src += ${origins.join(' ')}`) }
    const cli = join(root, 'node_modules', '@tauri-apps', 'cli', 'tauri.js')
    const result = spawnSync(process.execPath, [cli, ...withConfigArg(args, cspOverride(security, origins))], { stdio: 'inherit' })
    process.exit(result.status ?? 1)
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
    main().catch(e => {
        console.error(`[tauri.mjs] ${e instanceof Error ? e.message : String(e)}`)
        process.exit(1)
    })
}
