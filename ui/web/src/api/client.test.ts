import { afterEach, describe, expect, it, vi } from 'vitest'

import { ApiError, apiJson, getToken, setToken } from './client'

afterEach(() => {
  vi.unstubAllGlobals()
  window.localStorage.clear()
})

describe('apiJson', () => {
  it('uses same-origin cookies and returns JSON', async () => {
    const fetchMock = vi.fn().mockResolvedValue(
      new Response(JSON.stringify({ id: 'user-1' }), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      }),
    )
    vi.stubGlobal('fetch', fetchMock)

    await expect(apiJson('/api/me')).resolves.toEqual({ id: 'user-1' })
    expect(fetchMock).toHaveBeenCalledWith(
      '/api/me',
      expect.objectContaining({ credentials: 'same-origin' }),
    )
  })

  it('preserves the stable server error code', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue(
        new Response(
          // 真实服务端发的是嵌套信封 {error:{code,detail,next_step}}（error.rs）。
          JSON.stringify({
            error: {
              code: 'unauthorized',
              detail: '用户名或口令不正确。',
              next_step: '确认口令后重试。',
            },
          }),
          { status: 401, headers: { 'Content-Type': 'application/json' } },
        ),
      ),
    )

    await expect(apiJson('/api/auth/login')).rejects.toEqual(
      new ApiError(401, 'unauthorized', '用户名或口令不正确。', '确认口令后重试。'),
    )
  })

  it('reads Retry-After off the 429 response and keeps it off other statuses', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue(
        new Response(
          JSON.stringify({ error: { code: 'too_many_requests', detail: '登录尝试过于频繁。', next_step: '等 30 秒。' } }),
          { status: 429, headers: { 'Content-Type': 'application/json', 'Retry-After': '30' } },
        ),
      ),
    )
    await expect(apiJson('/api/auth/login', { method: 'POST' })).rejects.toMatchObject({
      status: 429,
      code: 'too_many_requests',
      retryAfterSeconds: 30,
    })

    // 401 就算带了这个头也不该被当成限流：按钮会被无意义地按住。
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue(
        new Response(
          JSON.stringify({ error: { code: 'unauthorized', detail: '凭据被拒', next_step: '' } }),
          { status: 401, headers: { 'Content-Type': 'application/json', 'Retry-After': '30' } },
        ),
      ),
    )
    await expect(apiJson('/api/auth/login', { method: 'POST' })).rejects.toMatchObject({
      status: 401,
      retryAfterSeconds: null,
    })
  })

  it('returns undefined for a successful empty response', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(new Response(null, { status: 204 })))
    await expect(apiJson('/api/auth/logout', { method: 'POST' })).resolves.toBeUndefined()
  })

  it('sends the stored access token as a bearer credential', async () => {
    const fetchMock = vi.fn().mockResolvedValue(
      new Response(JSON.stringify({ user_id: 'alice' }), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      }),
    )
    vi.stubGlobal('fetch', fetchMock)
    setToken('dev-token')

    await apiJson('/api/auth/me')
    const headers = (fetchMock.mock.calls[0]?.[1]?.headers ?? {}) as Headers
    expect(headers.get('Authorization')).toBe('Bearer dev-token')
  })

  it('forgets the token when it is cleared', () => {
    setToken('dev-token')
    setToken('')
    expect(getToken()).toBe('')
  })
})
