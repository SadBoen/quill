/**
 * 人格原文里提到、但 quill 还没做到**全部**能用的能力。
 *
 * **分两类，因为「没做到」有两种完全不同的含义**，混在一张表里只会误导：
 *
 * - `partial`：**有一部分真的能用了，但还缺一块**。技能包已经挂进对话的工具表
 *   （`ToolRegistry::with_skills`），每条 SKILL 用 `model_can_see` 告诉界面
 *   「模型这次看不看得见」；但 MCP 一条工具都还拿不到（协议层没接）。
 *   这种情况标「501」是撒谎 —— 501 的意思是路由不存在，而这些路由好好地活着。
 * - `not-implemented`：**路由根本不存在**，点了必然失败。
 *
 * 这张表的路由与状态码对应 `crates/quill-server/src/routes.rs`，内容对应
 * `crates/quill-server/src/tools.rs`。**改后端路由或挂接逻辑时必须同步改这里**，
 * 否则界面就在替后端说谎 —— 而且是那种没有任何报错会指向它的谎。
 */
export const CAPABILITY_GAPS = [
  {
    labelKey: 'chat.tools.mcp',
    fallback: 'MCP',
    route: 'GET /api/extensions/mcp',
    status: 'partial',
    detail: '配置能存能读；协议层（rmcp）未接，tools/list 还拿不到',
  },
  {
    labelKey: 'chat.tools.skills',
    fallback: '技能包',
    route: 'GET /api/extensions/skills',
    status: 'partial',
    detail: '已挂进对话工具表（每条带 model_can_see）；MCP 来的工具还没有',
  },
  {
    labelKey: 'chat.tools.plugins',
    fallback: '插件',
    route: 'GET /api/extensions/plugins',
    status: 'not-implemented',
    detail: '501',
  },
  {
    labelKey: 'chat.tools.cron',
    fallback: '定时任务',
    route: '/api/cron',
    status: 'not-implemented',
    detail: '路由未注册',
  },
] as const

type CapabilityGap = (typeof CAPABILITY_GAPS)[number]

type T = (key: string, options?: Record<string, unknown>) => string

/**
 * 状态文案。`partial` 与 `not-implemented` 用**不同的词**，不能共用一句
 * 「未接通」：MCP 与技能包的路由是 200，把它写成未接通等于告诉用户去接一个
 * 已经接好的东西。`partial` 后面跟的是**具体缺哪一环**，不是一个状态词。
 */
export function capabilityStatusLabel(gap: CapabilityGap, t: T): string {
  if (gap.status === 'partial') {
    return t('chat.tools.partial', {
      detail: gap.detail,
      defaultValue: '部分接通 · {{detail}}',
    })
  }
  return t('chat.tools.notReady', {
    state: gap.detail,
    defaultValue: '未接通 · {{state}}',
  })
}
