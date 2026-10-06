import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import i18n from '../i18n'
import { RESTORE_COMMAND } from './api'
import { WorkspacePage } from './WorkspacePage'

/**
 * 备份这块的红线：**页面上不能出现服务端没返回过的成功。**
 *
 * 之前这里只有两个真按钮：点「导出备份」必然 501（路由当时还没实现），
 * 而失败与成功在界面上长得一模一样 —— 用户只能看到一个点了没反应的东西。
 * 现在 export / verify 真的实现了，所以测试盯住三件事：
 *   1. 成功时显示的每个数字都是服务端给的（含**没被备份**的密钥材料）；
 *   2. 失败时一句成功的话都不许出现；
 *   3. 四种失败（400 / 409 / 503 / 404）问的是四个不同问题，不能合成一句。
 *
 * 另外：还原**没有**按钮，只有「先停掉服务端 + 真实 CLI 命令」的说明。
 */

const BACKUP = {
  name: 'backup-2026-10-06',
  dest: '/var/lib/quill-backup/backup-2026-10-06',
  db_sha256: '9f2c1ab4d5e6f708192a3b4c5d6e7f8091a2b3c4d5e6f708192a3b4c5d6e7f809',
  files: 128,
  total_bytes: 4718592,
  excluded: [
    { rel: 'keys/api.key', reason: '密钥材料不进备份' },
    { rel: 'keys/mcp.token', reason: '密钥材料不进备份' },
  ],
}

const VERIFY = {
  name: 'backup-2026-10-06',
  ok: true as const,
  files: 128,
  total_bytes: 4718592,
  db_sha256: BACKUP.db_sha256,
}

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  })
}

/** 服务端错误信封，字段名见 `api/client.ts` 顶部的注释。 */
function serverError(status: number, code: string, detail: string, nextStep: string): Response {
  return json({ error: { code, detail, next_step: nextStep } }, status)
}

/**
 * 统一入口。fetch 按 URL 分派：export / verify 各自的成败由用例决定，
 * 升级那两条路由在本页仍是 501（所以回 501，与后端现状一致）。
 */
function renderPage(handlers: Record<string, () => Promise<Response>>): {
  requests: Array<{ url: string; body: unknown }>
} {
  const requests: Array<{ url: string; body: unknown }> = []
  vi.stubGlobal(
    'fetch',
    vi.fn(async (url: string, init?: RequestInit) => {
      const handler = handlers[url]
      requests.push({ url, body: init?.body ? JSON.parse(String(init.body)) : undefined })
      if (!handler) {
        return serverError(501, 'not_implemented', `${url} 尚未实现。`, '等待后端实现该路由。')
      }
      return handler()
    }),
  )
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  render(
    <QueryClientProvider client={client}>
      <WorkspacePage />
    </QueryClientProvider>,
  )
  return { requests }
}

const EXPORT_URL = '/api/backup/export'
const VERIFY_URL = '/api/backup/verify'

function clickExport(): void {
  fireEvent.click(screen.getByRole('button', { name: '导出备份' }))
}

function clickVerify(): void {
  fireEvent.click(screen.getByRole('button', { name: '校验备份' }))
}

beforeEach(async () => {
  await i18n.changeLanguage('zh-CN')
  vi.unstubAllGlobals()
})

afterEach(() => {
  cleanup()
})

describe('导出备份：成功时显示服务端给的每一个字段', () => {
  it('目标目录、数据库摘要、文件数、总字节、以及被排除的密钥材料都在页面上', async () => {
    renderPage({ [EXPORT_URL]: () => Promise.resolve(json(BACKUP, 201)) })

    clickExport()

    const panel = await screen.findByTestId('backup-export-result')
    expect(panel).toHaveTextContent('导出完成')
    expect(screen.getByTestId('backup-dest')).toHaveTextContent(BACKUP.dest)
    expect(panel).toHaveTextContent(BACKUP.db_sha256)
    expect(screen.getByTestId('backup-files')).toHaveTextContent(String(BACKUP.files))
    expect(screen.getByTestId('backup-total-bytes')).toHaveTextContent(String(BACKUP.total_bytes))

    // 排除项是这个接口存在的理由：不列出来，用户会以为这份备份是全的。
    const excluded = screen.getByTestId('backup-excluded')
    for (const entry of BACKUP.excluded) {
      expect(excluded).toHaveTextContent(entry.rel)
      expect(excluded).toHaveTextContent(entry.reason)
    }
  })

  it('请求体就是约定的 { name }，name 是相对名而不是路径', async () => {
    const { requests } = renderPage({ [EXPORT_URL]: () => Promise.resolve(json(BACKUP, 201)) })

    clickExport()

    await waitFor(() => expect(requests.some((r) => r.url === EXPORT_URL)).toBe(true))
    expect(requests.find((r) => r.url === EXPORT_URL)?.body).toEqual({ name: 'backup-2026-10-06' })
  })

  it('服务端说成功但字段残缺时，不许渲染成「导出完成」', async () => {
    // 摘要被截成空串、文件数变成 null —— 这种情况以前会被照单全收地显示成功。
    renderPage({ [EXPORT_URL]: () => Promise.resolve(json({ ...BACKUP, db_sha256: '', files: null }, 201)) })

    clickExport()

    expect(await screen.findByTestId('backup-error-unreadable')).toBeInTheDocument()
    expect(screen.queryByTestId('backup-export-result')).toBeNull()
    expect(screen.queryByText('导出完成')).toBeNull()
  })
})

describe('失败时说清楚是哪一种失败', () => {
  it('400：名字不合法 → 让用户改成一个纯名字', async () => {
    renderPage({
      [EXPORT_URL]: () => Promise.resolve(serverError(400, 'invalid_backup_name', '备份名不合法。', '改成相对名。')),
    })

    clickExport()

    const notice = await screen.findByTestId('backup-error-invalid-name')
    expect(notice).toHaveTextContent('改成一个纯名字')
    // 服务端原文也要留着。
    expect(await screen.findByRole('alert')).toHaveTextContent('备份名不合法。')
  })

  it('409：目标已存在且非空 → 让用户换个名字，和 400 的话不一样', async () => {
    renderPage({
      [EXPORT_URL]: () => Promise.resolve(serverError(409, 'backup_dest_not_empty', '目标目录不是空的。', '换个名字。')),
    })

    clickExport()

    const notice = await screen.findByTestId('backup-error-dest-not-empty')
    expect(notice).toHaveTextContent('换一个没用过的备份名')
    // 409 与 400 问的不是同一个问题，句子里不许互相冒充。
    expect(notice).not.toHaveTextContent('纯名字')
    expect(screen.queryByTestId('backup-error-invalid-name')).toBeNull()
  })

  it('503：备份源不可用 → 先去体检，不是「备份完成」', async () => {
    renderPage({
      [EXPORT_URL]: () =>
        Promise.resolve(serverError(503, 'backup_source_unavailable', '备份源不可用。', '先跑 quill doctor。')),
    })

    clickExport()

    const notice = await screen.findByTestId('backup-error-source-unavailable')
    expect(notice).toHaveTextContent('quill doctor')
    expect(notice).toHaveTextContent('这不是「备份完成」')
  })

  it('导出失败时页面上没有任何成功状态', async () => {
    // 这就是「按钮点了像坏了」的那条：以前失败之后界面上什么提示都没有。
    renderPage({
      [EXPORT_URL]: () =>
        Promise.resolve(serverError(503, 'backup_source_unavailable', '备份源不可用。', '先跑 quill doctor。')),
    })

    clickExport()

    await screen.findByRole('alert')
    expect(screen.queryByTestId('backup-export-result')).toBeNull()
    expect(screen.queryByText('导出完成')).toBeNull()
    expect(screen.queryByTestId('backup-files')).toBeNull()
  })
})

describe('校验备份', () => {
  it('成功时显示 ok 与文件数', async () => {
    renderPage({ [VERIFY_URL]: () => Promise.resolve(json(VERIFY)) })

    clickVerify()

    const panel = await screen.findByTestId('backup-verify-result')
    expect(panel).toHaveTextContent('校验通过')
    expect(screen.getByTestId('verify-ok')).toHaveTextContent('true')
    expect(screen.getByTestId('verify-files')).toHaveTextContent(String(VERIFY.files))
  })

  it('422：备份自己坏了 → 说「别用这份」，绝不能说「换个名字」', async () => {
    // 这条是修掉的真缺陷：422 曾经被并进 409，于是校验抓到损坏时，
    // 界面叫用户去改备份名 —— 改名字对一份坏掉的文件不会有任何作用。
    renderPage({
      [VERIFY_URL]: () =>
        Promise.resolve(serverError(422, 'unprocessable', '备份 badman 校验未通过：1 处内容与清单不符。', '这份备份不可用于还原。')),
    })

    clickVerify()

    const notice = await screen.findByTestId('backup-error-backup-corrupt')
    expect(notice).toHaveTextContent('不要拿去还原')
    expect(notice).not.toHaveTextContent('换一个没用过的备份名')
    expect(screen.queryByTestId('backup-error-dest-not-empty')).toBeNull()
    expect(screen.queryByTestId('backup-verify-result')).toBeNull()
    expect(screen.queryByText('校验通过')).toBeNull()
  })

  it('校验失败时显示的是错误，不是校验通过', async () => {
    renderPage({
      [VERIFY_URL]: () => Promise.resolve(serverError(404, 'backup_not_found', '没有这份备份。', '核对名字拼写。')),
    })

    clickVerify()

    expect(await screen.findByTestId('backup-error-unknown-backup')).toHaveTextContent('核对名字拼写')
    expect(screen.queryByTestId('backup-verify-result')).toBeNull()
    expect(screen.queryByText('校验通过')).toBeNull()
  })

  it('校验回来的 ok 不为 true 时不算通过', async () => {
    renderPage({ [VERIFY_URL]: () => Promise.resolve(json({ ...VERIFY, ok: false })) })

    clickVerify()

    expect(await screen.findByTestId('backup-error-unreadable')).toBeInTheDocument()
    expect(screen.queryByTestId('backup-verify-result')).toBeNull()
  })

  it('校验走的是同一个相对名', async () => {
    const { requests } = renderPage({ [VERIFY_URL]: () => Promise.resolve(json(VERIFY)) })

    clickVerify()

    await waitFor(() => expect(requests.some((r) => r.url === VERIFY_URL)).toBe(true))
    expect(requests.find((r) => r.url === VERIFY_URL)?.body).toEqual({ name: 'backup-2026-10-06' })
  })
})

describe('还原：只有说明，没有按钮', () => {
  it('说明里带着真实的 CLI 命令', async () => {
    renderPage({})

    const note = await screen.findByTestId('restore-note')
    expect(note).toHaveTextContent('停掉服务端')
    // 命令原文取自 crates/quill-cli/src/cmd_backup.rs。
    expect(note).toHaveTextContent(RESTORE_COMMAND)
    expect(RESTORE_COMMAND).toContain('quill restore')
    expect(RESTORE_COMMAND).toContain('--yes')
  })

  it('页面上没有「还原」按钮，也没有假装能成功的还原动作', async () => {
    renderPage({})

    await screen.findByTestId('restore-note')
    const names = screen.getAllByRole('button').map((b) => b.textContent)
    expect(names.some((n) => n?.includes('还原'))).toBe(false)
    expect(names.some((n) => n?.includes('restore') || n?.includes('Restore'))).toBe(false)
  })

  it('备份名为空时不发请求，也不显示任何结果', async () => {
    const { requests } = renderPage({ [EXPORT_URL]: () => Promise.resolve(json(BACKUP, 201)) })

    fireEvent.change(screen.getByLabelText('备份名'), { target: { value: '  ' } })

    expect(screen.getByRole('button', { name: '导出备份' })).toBeDisabled()
    expect(screen.getByRole('button', { name: '校验备份' })).toBeDisabled()
    expect(requests.filter((r) => r.url === EXPORT_URL)).toHaveLength(0)
    expect(screen.queryByTestId('backup-export-result')).toBeNull()
  })
})