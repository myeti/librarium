#!/usr/bin/env node
/**
 * Cross-checks frontend `invoke('cmd', { args })` calls against the Rust
 * `#[tauri::command]` signatures: unknown commands, unknown argument names
 * and missing required arguments are reported. Tauri maps camelCase JS
 * argument names to snake_case Rust parameters; `app`, `state`, `window` and
 * other injected parameters are ignored.
 *
 * Usage: node scripts/check-ipc.mjs   (exit code 1 on mismatch)
 */
import { readFileSync, readdirSync, statSync } from 'node:fs'
import { join, sep } from 'node:path'
import { fileURLToPath } from 'node:url'

// fileURLToPath (not URL.pathname) so the script also works on Windows.
const root = fileURLToPath(new URL('..', import.meta.url))
const rel = p => p.replace(root, '').split(sep).join('/')

function walk(dir, exts, out = []) {
    for (const name of readdirSync(dir)) {
        const p = join(dir, name)
        if (statSync(p).isDirectory()) { walk(p, exts, out) }
        else if (exts.some(e => p.endsWith(e)) && !p.includes('.test.')) { out.push(p) }
    }
    return out
}

const toCamel = s => s.replace(/_([a-z])/g, (_, c) => c.toUpperCase())
const injected = /^(AppHandle|State<|tauri::State|Window|tauri::Window|WebviewWindow)/

// ── Rust commands ───────────────────────────────────────────────────────────
const commands = new Map()
for (const file of walk(join(root, 'src-tauri/src'), ['.rs'])) {
    const src = readFileSync(file, 'utf8')
    const re = /#\[tauri::command\][^\n]*\n\s*pub\s+(?:async\s+)?fn\s+(\w+)\s*(?:<[^>]*>)?\s*\(([\s\S]*?)\)\s*(?:->|\{)/g
    for (const m of src.matchAll(re)) {
        const params = []
        for (const raw of m[2].split(/,(?![^<]*>)/)) {
            const p = raw.trim()
            if (!p) { continue }
            const [name, ...ty] = p.replace(/^mut\s+/, '').split(':')
            const type = ty.join(':').trim()
            if (injected.test(type) || name.startsWith('_')) { continue }
            params.push({ name: toCamel(name.trim()), optional: type.startsWith('Option<') })
        }
        commands.set(m[1], { params, file: rel(file) })
    }
}

// ── Frontend calls ──────────────────────────────────────────────────────────
let problems = 0
const used = new Set()
for (const file of walk(join(root, 'src'), ['.ts', '.vue'])) {
    const src = readFileSync(file, 'utf8')
    // `invoke<Record<string, number>>(...)`: generics may nest, so match lazily up to `>(`.
    const re = /invoke(?:<[^()]*?>)?\(\s*'(\w+)'\s*(,\s*\{([^}]*)\}|,\s*[\w.]+)?/g
    for (const m of src.matchAll(re)) {
        const [, cmd, second, body] = m
        // Arguments passed as a variable (`invoke('cmd', args)`) can't be checked statically.
        const opaque = second !== undefined && body === undefined
        used.add(cmd)
        const line = src.slice(0, m.index).split('\n').length
        const where = `${rel(file)}:${line}`
        const def = commands.get(cmd)
        if (!def) { console.log(`✗ ${where}: unknown command '${cmd}'`); problems++; continue }
        const keys = (body ?? '').split(',')
            .map(s => s.trim().split(':')[0].trim().replace(/^\.\.\..*/, ''))
            .filter(k => /^\w+$/.test(k))
        const spread = opaque || /\.\.\./.test(body ?? '')
        for (const k of keys) {
            if (!def.params.some(p => p.name === k)) { console.log(`✗ ${where}: '${cmd}' has no argument '${k}'`); problems++ }
        }
        if (!spread) {
            for (const p of def.params) {
                if (!p.optional && !keys.includes(p.name)) { console.log(`✗ ${where}: '${cmd}' is missing required '${p.name}'`); problems++ }
            }
        }
    }
}
for (const [name, def] of commands) {
    if (!used.has(name)) { console.log(`· ${name} (${def.file}) is never invoked from the frontend`) }
}
console.log(problems ? `\n${problems} IPC mismatch(es)` : `\nIPC OK: ${used.size} commands used, ${commands.size} defined`)
process.exit(problems ? 1 : 0)
