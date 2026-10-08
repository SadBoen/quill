import { describe, expect, it } from 'vitest'

import { CAPABILITY_GAPS, capabilityStatusLabel } from './capabilityGaps'

/** 假装 react-i18next 的 t：直接用 defaultValue 套占位符。 */
const t = (key: string, options?: Record<string, unknown>): string => {
  const map: Record<string, string> = {
    'chat.tools.notReady': '未接通 · {{state}}',
    'chat.tools.partial': '部分接通 · {{detail}}',
  }
  return (map[key] ?? key).replace(/\{\{(\w+)\}\}/g, (_, name: string) => String(options?.[name] ?? ''))
}

/**
 * 这张表会直接显示给用户，说错就是在骗人。这几条断言盯的就是「别把已经
 * 接通的东西说成没接」。
 */
describe('能力缺口表必须与后端路由对齐', () => {
  it('MCP 与技能包绝不能标成 501 —— 它们的路由是 200', () => {
    // 这两条路由在 crates/quill-server/src/routes.rs 里是真的，200。
    // 标 501 会让用户去「接」一个早就接好的东西。
    for (const route of ['GET /api/extensions/mcp', 'GET /api/extensions/skills']) {
      const gap = CAPABILITY_GAPS.find((item) => item.route === route)
      expect(gap, `${route} 不该再出现在「没做到能用」的表里`).toBeDefined()
      expect(gap?.status, `${route} 不该标成 not-implemented`).toBe('partial')
      const label = capabilityStatusLabel(gap!, t)
      expect(label).toMatch(/^部分接通/)
      expect(label, `${route} 的文案里不许出现 501`).not.toContain('501')
    }
  })

  it('路由压根不存在的那一条才是「未接通」', () => {
    // `/api/cron` 在 2026-10-09 接通了（Q042），所以它**不在**这张表里了 ——
    // 表里剩下的只有「路由登记了但处理函数没实现」的插件那条。
    for (const route of ['GET /api/extensions/plugins']) {
      const gap = CAPABILITY_GAPS.find((item) => item.route === route)
      expect(gap?.status).toBe('not-implemented')
      expect(capabilityStatusLabel(gap!, t)).toMatch(/^未接通/)
    }
    // 定时任务不该再出现在「没做到能用」的表里（它现在是真后端）。
    // 用 `map` 到 `string[]` 再比：直接拿 `item.route` 比会被 TS 收窄成「不可能相等」。
    const routes: readonly string[] = CAPABILITY_GAPS.map((item) => item.route)
    expect(routes).not.toContain('/api/cron')
  })

  it('每一行都要有名字和路由', () => {
    for (const gap of CAPABILITY_GAPS) {
      expect(gap.labelKey, `${gap.route} 缺 i18n key`).toBeTruthy()
      expect(gap.fallback, `${gap.route} 缺中文兜底文案`).toBeTruthy()
      expect(gap.detail, `${gap.route} 缺状态说明`).toBeTruthy()
      // 已注册的路由带 HTTP 方法；没注册的那条只写路径。
      // 这里只保证「看起来像个路由」，不强行统一写法。
      expect(gap.route, `${gap.route} 不像个路由`).toMatch(/^(GET|POST|PATCH|PUT|DELETE) |^\/api\//)
    }
  })

  it('部分接通的每一行都要说清「哪一环没接」，而不只是给个状态词', () => {
    // 「部分接通」四个字本身没用：用户不知道要等协议层还是等工具表。
    // 只写状态码的话，两种情况在界面上长得一模一样。
    // 措辞允许几种（未接 / 尚未 / 还没有 / 还没），但必须**指名缺的是哪一环**。
    for (const gap of CAPABILITY_GAPS.filter((item) => item.status === 'partial')) {
      expect(gap.detail, `${gap.route} 的 detail 没说要接什么`).toMatch(/未接|尚未|没有|还没/)
      expect(gap.detail.length, `${gap.route} 的 detail 太短，说不清差在哪`).toBeGreaterThan(6)
    }
  })

  it('技能包已经挂进对话工具表了 —— 文案不许再说「尚未挂进」', () => {
    // `ToolRegistry::with_skills` 已经把启用的 SKILL 挂进对话的工具表
    // （crates/quill-server/src/tools.rs），每条 SKILL 还能通过
    // `model_can_see` 告诉界面「模型这次看不看得见」。
    // 文案停留在旧状态，就是在把已经能用的东西说成不能用 —— 和把没用的
    // 说成能用一样，是这个项目最不能出的错。
    const skills = CAPABILITY_GAPS.find((item) => item.route === 'GET /api/extensions/skills')
    expect(skills).toBeDefined()
    const detail = skills!.detail
    expect(detail, '技能包已挂进工具表，不该再说尚未挂进').not.toMatch(/尚未挂进|还调不到/)
    expect(detail, '要说清已经挂进工具表了').toMatch(/挂进/)
  })

  it('MCP 仍然标 partial，但缺口已经换成「非 stdio 传输还没铺」', () => {
    // 这一条断言改过两次，每次都是因为**真的接完了一层**：
    //   1. 2026-10-06 之前盯的是「缺协议层」；
    //   2. 协议层真的握手并 tools/list 之后，缺的是**挂进对话工具表**这一步；
    //   3. `ToolRegistry::with_mcp_tools` 与 `tools/call` 都落地之后，
    //      剩下的缺口是**传输方式**：`streamable_http` 与 `sse` 在
    //      `mcp_client::discover` 里明确报「还没铺」，这一层不假装连过。
    // 每一层接完都要回来改这条 —— 拿旧缺口当现状写，就是把能用的说成不能用。
    const mcp = CAPABILITY_GAPS.find((item) => item.route === 'GET /api/extensions/mcp')
    expect(mcp?.status).toBe('partial')
    // 已经做好的那几步必须说清楚，否则用户会以为一条工具都拿不到。
    expect(mcp!.detail, '要说清 stdio 协议层已经真的握手并 tools/list 了').toMatch(/tools\/list/)
    expect(mcp!.detail, '要说清工具真的挂进对话工具表了').toMatch(/挂进对话工具表/)
    // 现在真正的缺口是传输方式，不许再拿「挂进工具表」当缺口。
    expect(mcp!.detail, '工具已挂进工具表，不该再拿这一步当缺口').not.toMatch(/还没挂进对话/)
    // 已经做好的步骤不许被说成没接。
    expect(mcp!.detail, '协议层与挂载都已落地').not.toMatch(/协议层（未接|没接|还没）/)
    expect(mcp!.detail, '要说清剩下的是哪些传输').toMatch(/streamable_http|sse/)
  })
})
