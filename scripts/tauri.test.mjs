import { describe, expect, it } from 'vitest'
import { cspOverride, serverOrigins, withConfigArg, withConnectOrigins } from './tauri.mjs'

const CSP = "default-src 'self'; connect-src 'self' ipc: https://openlibrary.org; object-src 'none';"

describe('scripts/tauri.mjs: the build\'s servers in the CSP', () => {
    it('takes the origin of each server setting, once', () => {
        expect(serverOrigins({
            VITE_API_URL: 'https://api.example.org/v1/',
            VITE_UMAMI_URL: 'https://stats.example.org:8443/api/send',
        })).toEqual(['https://api.example.org', 'https://stats.example.org:8443'])
        expect(serverOrigins({ VITE_API_URL: 'https://same.example.org/a', VITE_UMAMI_URL: 'https://same.example.org/b' }))
            .toEqual(['https://same.example.org'])
        expect(serverOrigins({ VITE_API_URL: '  ', VITE_API_KEY: 'not a server' })).toEqual([])
    })

    it('refuses what isn\'t an http(s) URL (it would land in the CSP)', () => {
        expect(() => serverOrigins({ VITE_API_URL: "https://x.org; script-src 'unsafe-inline'" })).toThrow(/URL/)
        expect(() => serverOrigins({ VITE_API_URL: 'javascript:alert(1)' })).toThrow(/http/)
        expect(() => serverOrigins({ VITE_UMAMI_URL: 'not a url' })).toThrow(/URL/)
    })

    it('adds the origins to connect-src only', () => {
        expect(withConnectOrigins(CSP, ['https://api.example.org']))
            .toBe("default-src 'self'; connect-src 'self' ipc: https://openlibrary.org https://api.example.org; object-src 'none';")
        expect(withConnectOrigins(CSP, [])).toBe(CSP)
        expect(() => withConnectOrigins("default-src 'self'", ['https://a.org'])).toThrow(/connect-src/)
    })

    it('passes the override before cargo\'s arguments, and nothing without servers', () => {
        const override = cspOverride({ csp: CSP, devCsp: CSP }, ['https://a.org'])
        expect(override?.app.security.devCsp).toContain('https://a.org')
        const args = withConfigArg(['build', '--ci', '--', '--locked'], override)
        expect(args.slice(0, 3)).toEqual(['build', '--ci', '--config'])
        expect(JSON.parse(args[3])).toEqual(override)
        expect(args.slice(4)).toEqual(['--', '--locked'])
        expect(withConfigArg(['dev'], cspOverride({ csp: CSP, devCsp: CSP }, []))).toEqual(['dev'])
    })
})
