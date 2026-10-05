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

  it('路由压根不存在的那两条才是「未接通」', () => {
    for (const route of ['GET /api/extensions/plugins', '/api/cron']) {
      const gap = CAPABILITY_GAPS.find((item) => item.route === route)
      expect(gap?.status).toBe('not-implemented')
      expect(capabilityStatusLabel(gap!, t)).toMatch(/^未接通/)
    }
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
    for (const gap of CAPABILITY_GAPS.filter((item) => item.status === 'partial')) {
      expect(gap.detail, `${gap.route} 的 detail 没说要接什么`).toMatch(/未接|尚未/)
      expect(gap.detail.length, `${gap.route} 的 detail 太短，说不清差在哪`).toBeGreaterThan(6)
    }
  })
})
