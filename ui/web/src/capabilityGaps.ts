/**
 * 人格原文里提到、但 quill 还没做到**全部**能用的能力。
 *
 * **分两类，因为「没做到」有两种完全不同的含义**，混在一张表里只会误导：
 *
 * - `partial`：**有一部分真的能用了，但还缺一块**。技能包已经挂进对话的工具表
 *   （`ToolRegistry::with_skills`），每条 SKILL 用 `model_can_see` 告诉界面
 *   「模型这次看不看得见」；MCP 的 stdio 协议层也已经真的握手并 `tools/list`
 *   （`mcp_client::probe`，每台带真实的 `connected` / `tool_count` / 失败原因），
 *   但那些工具**还没有挂进对话的工具表**，所以模型这一轮仍然调不到。
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
    // 「还差什么」必须是真的还差着。stdio 与 tools/call 都通了、工具也真的挂进
    // 对话工具表了，所以剩下的缺口只有传输方式：streamable_http / sse 还没铺。
    detail: 'stdio 真的 initialize + tools/list + tools/call，工具已挂进对话工具表；streamable_http 与 sse 这两种传输还没铺',
  },
  {
    labelKey: 'chat.tools.skills',
    fallback: '技能包',
    route: 'GET /api/extensions/skills',
    status: 'partial',
    // SKILL 本身已经通了（挂进工具表 + 每条带 model_can_see）。partial 成立
    // 的理由换成了**同族的另外几条路由**：`PATCH /api/extensions/skills/{name}`
    // 与 `bundle/import`、`bundle/export` 在 routes.rs 里都还没登记。
    // **点名是哪几条**，而不是甩一个状态码：这一行自己挂的 GET/POST/DELETE
    // 都是 200，写「501」会让人以为技能包这条路整体没通。
    detail: '已挂进对话工具表（每条带 model_can_see）；但 PATCH /api/extensions/skills/{name} 与 bundle 导入导出这三条还没登记',
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
