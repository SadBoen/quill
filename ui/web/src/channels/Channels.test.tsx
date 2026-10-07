import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { MemoryRouter } from 'react-router-dom'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import i18n from '../i18n'
import { ChannelsPage } from './Channels'

/**
 * 通道页的判据。
 *
 * 这页的坏处比别处隐蔽：它画的每个按钮都能点，所以「编译过、渲染得出来」
 * 几乎不能证明它是对的。真正要钉的是三件：
 *
 * 1. **凭据不进浏览器**。这是整条链路的安全边界。前端类型里没有 token 字段，
 *    渲染时也不该有任何地方把它显示出来 —— 源码里出现 `.token` 就是红线。
 * 2. **默认关着**。新连上的通道是「只有名单里的人」且名单为空。
 *    界面上必须能一眼看出「现在谁都不能私聊它」，而不是默认放开。
 * 3. **扫码失败要有人话**。iLink 挂掉、扫码过期、网络不通都得说清楚，
 *    静默失败会让人以为是自己手机的问题。
 */

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  })
}

function serverError(status: number, code: string, detail: string, nextStep: string): Response {
  return json({ error: { code, detail, next_step: nextStep } }, status)
}

const EMPTY = { channels: [], supported: ['weixin'] }

const CONFIGURED = {
  channels: [
    {
      channel_id: 'weixin-main',
      kind: 'weixin',
      name: '我的微信',
      enabled: false,
      configured: true,
      dm_policy: 'allowlist',
      accounts: [{ account_id: 'acc-1@im.bot', account_name: '小王', configured: true }],
    },
  ],
  supported: ['weixin'],
}

function mount() {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return render(
    <MemoryRouter>
      <QueryClientProvider client={qc}>
        <ChannelsPage />
      </QueryClientProvider>
    </MemoryRouter>,
  )
}

beforeEach(async () => {
  await i18n.changeLanguage('zh')
  vi.restoreAllMocks()
})

describe('通道页', () => {
  it('空列表也要画出通道卡片，而不是一片空白', async () => {
    vi.stubGlobal('fetch', vi.fn(async () => json(EMPTY)))
    mount()
    // 等**卡片**而不是等标题：标题在加载中就已经在页面上了，
    // 等它等于什么都没等，断言会挂在一个跟行为无关的时机上。
    await waitFor(() => expect(screen.getByText('微信（个人号）')).toBeTruthy())
    // 没有通道 ≠ 没有这一页：用户得看到「还没连」这个事实
    expect(screen.getByRole('button', { name: '微信扫码连接' })).toBeTruthy()
  })

  it('已配置但未启用时要说清「消息不会被接收」', async () => {
    vi.stubGlobal('fetch', vi.fn(async () => json(CONFIGURED)))
    mount()
    await waitFor(() => expect(screen.getByText('已经连上但没启用：消息不会被接收。')).toBeTruthy())
    // 账号 id 是非密标识，可以显示
    expect(screen.getByText('acc-1@im.bot')).toBeTruthy()
  })

  it('默认准入策略显示为「只有名单里的人」，不是「任何人」', async () => {
    vi.stubGlobal('fetch', vi.fn(async () => json(CONFIGURED)))
    mount()
    await waitFor(() => expect(screen.getByText('谁能私聊')).toBeTruthy())
    const select = screen.getByDisplayValue('只有名单里的人（默认）') as HTMLSelectElement
    expect(select.value).toBe('allowlist')
    // 并且要说清空名单 = 谁都进不来
    expect(screen.getByText(/名单一开始是空的/)).toBeTruthy()
  })

  it('点「重新扫码」会先取二维码', async () => {
    const calls: string[] = []
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        calls.push(String(input))
        if (String(input).includes('qrcode/generate')) {
          return json({ qrcode_token: 'q1', qrcode_url: '', expires_at: 123 })
        }
        return json(CONFIGURED)
      }),
    )
    mount()
    await waitFor(() => expect(screen.getByRole('button', { name: '重新扫码' })).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: '重新扫码' }))
    await waitFor(() =>
      expect(calls.some((c) => c.includes('/api/channels/weixin/qrcode/generate'))).toBe(true),
    )
    await waitFor(() =>
      expect(screen.getByText('请用微信扫码，并在手机上确认。')).toBeTruthy(),
    )
  })

  it('扫码接口报错时给出人话，不是空白页', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        if (String(input).includes('qrcode/generate')) {
          return serverError(503, 'service_unavailable', '微信通道调用失败：连不上 ilinkai', '检查网络后重试')
        }
        return json(CONFIGURED)
      }),
    )
    mount()
    await waitFor(() => expect(screen.getByRole('button', { name: '重新扫码' })).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: '重新扫码' }))
    await waitFor(() => expect(screen.getByText(/微信通道调用失败/)).toBeTruthy())
  })

  it('列表接口挂了要说清是服务端的问题', async () => {
    vi.stubGlobal('fetch', vi.fn(async () => serverError(503, 'service_unavailable', '存储不可用', '查看 doctor')))
    mount()
    await waitFor(() => expect(screen.getByText(/存储不可用/)).toBeTruthy())
  })

  it('源码里不出现 .token —— 凭据不进浏览器这条不能靠自觉', async () => {
    // 静态导入源文件：类型里没有 token 不代表渲染时不会有人顺手读它。
    const src = await import('./Channels.tsx?raw')
    expect(src.default).not.toMatch(/\.token\b/)
    expect(src.default).not.toMatch(/bot_token/)
  })

  it('前端类型里没有凭据字段', async () => {
    const src = await import('./api.ts?raw')
    expect(src.default).not.toMatch(/^\s*token\??\s*:/m)
    expect(src.default).not.toMatch(/^\s*bot_token\??\s*:/m)
  })
})