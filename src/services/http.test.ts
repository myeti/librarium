import { beforeEach, describe, expect, it, vi } from 'vitest'

vi.mock('@tauri-apps/api/core', () => import('@/test/tauri'))
import { backendError, mockInvoke } from '@/test/tauri'
import { httpGet, httpGetJson, HttpFormatError, HttpStatusError } from './http'

const ok = (body: unknown, status = 200) => ({
    status, contentType: 'application/json', body: JSON.stringify(body), finalUrl: 'https://x.test/',
})

describe('http service', () => {
    beforeEach(() => mockInvoke.reset())

    it('calls http_get with the documented camelCase arguments', async () => {
        mockInvoke.on('http_get', () => ok({}))
        await httpGet('https://x.test/a', { headers: { Accept: 'application/json' }, maxBytes: 1000 })
        expect(mockInvoke.callsTo('http_get')[0].args).toEqual({
            url: 'https://x.test/a', headers: { Accept: 'application/json' }, maxBytes: 1000,
        })
    })

    it('parses JSON bodies', async () => {
        mockInvoke.on('http_get', () => ok({ a: 1 }))
        await expect(httpGetJson('https://x.test/')).resolves.toEqual({ a: 1 })
    })

    it('rejects non-2xx statuses instead of returning an empty result', async () => {
        mockInvoke.on('http_get', () => ok({ error: 'nope' }, 503))
        await expect(httpGetJson('https://x.test/')).rejects.toBeInstanceOf(HttpStatusError)
    })

    it('rejects bodies that are not JSON with a format error', async () => {
        mockInvoke.on('http_get', () => ({ status: 200, contentType: 'application/atom+xml', body: '<feed/>', finalUrl: 'https://x.test/' }))
        await expect(httpGetJson('https://x.test/')).rejects.toBeInstanceOf(HttpFormatError)
    })

    it('rejects with AbortError when the signal is aborted', async () => {
        mockInvoke.on('http_get', () => ok({}))
        const ctrl = new AbortController()
        ctrl.abort()
        await expect(httpGetJson('https://x.test/', { signal: ctrl.signal })).rejects.toMatchObject({ name: 'AbortError' })
        expect(mockInvoke.callsTo('http_get')).toHaveLength(0)
    })

    it('propagates network errors from Rust', async () => {
        mockInvoke.on('http_get', () => backendError('connection refused'))
        await expect(httpGetJson('https://x.test/')).rejects.toBe('connection refused')
    })
})
