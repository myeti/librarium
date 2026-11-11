/**
 * Test helper: a programmable stand-in for Tauri's `invoke`.
 *
 * Usage in a test file:
 *
 *     vi.mock('@tauri-apps/api/core', () => import('@/test/tauri'))
 *     import { mockInvoke } from '@/test/tauri'
 *     mockInvoke.on('scan_library', () => [])
 *
 * Unhandled commands reject, so a test fails loudly when code calls the
 * backend in a way the test didn't anticipate.
 */
type Handler = (args: Record<string, unknown>) => unknown

const handlers = new Map<string, Handler>()
const calls: { cmd: string, args: Record<string, unknown> }[] = []

export const mockInvoke = {
    on(cmd: string, handler: Handler) { handlers.set(cmd, handler) },
    calls,
    callsTo(cmd: string) { return calls.filter(c => c.cmd === cmd) },
    reset() { handlers.clear(); calls.length = 0 },
}

export async function invoke<T>(cmd: string, args: Record<string, unknown> = {}): Promise<T> {
    calls.push({ cmd, args })
    const handler = handlers.get(cmd)
    if (!handler) { throw new Error(`Unexpected invoke('${cmd}')`) }
    return await handler(args) as T
}

/**
 * Makes a mocked command fail the way a real one does: Tauri rejects with
 * the value the backend returned (a message string, or a `{ code, message }`
 * object), not with an `Error`. Use it in a handler:
 *
 *     mockInvoke.on('delete_books', () => backendError('file is locked'))
 */
export function backendError(value: unknown): never {
    throw value
}

export function convertFileSrc(path: string): string {
    return `asset://localhost/${encodeURIComponent(path)}`
}
