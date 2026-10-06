/**
 * 待办的**唯一声明来源**。
 *
 * 为什么要它存在：`BACKLOG.md` 与 `MILESTONES.md` 曾把「事实」抄进散文 ——
 * 测试数写成 280（实际 1175）、状态用 emoji 手打。这些东西没有对账机制，
 * 代码一变就腐烂，而腐烂会连累整份文档的可信度。
 *
 * 所以这里只放**声明**：每条待办自己说明「怎么算做完」。真不真，由
 * `scripts/status.mjs` 当场跑出来 —— 人不打状态，机器算。
 *
 * 散文（为什么这么做、踩过什么坑）仍然留在 BACKLOG.md，但那里**不许**再出现
 * 任何测试数字或状态标记。判据与事实混在一段话里是 Diátaxis 说的那类伤害：
 * 混着写，改的时候必然顾此失彼。
 *
 * 判据种类：
 *   test   —— 某条具名测试必须存在且通过。绑定不存在的名字 = 恒为真的假判据，
 *             所以 status.mjs 会显式区分「不存在」与「没通过」。
 *   cmd    —— 一条命令必须退出 0。
 *   absent —— 某个路径必须**不存在**（用于「我们删掉了它」这类判据）。
 *   manual —— 只能人工验证。**它必须显眼**：状态里单列一栏，不许混进「已通过」。
 *             藏起来的未验证比明摆着危险得多。
 *
 * 新增一条待办时，`status.mjs` 会校验 verify 声明本身是否成立（见该脚本的
 * `--self-check`）。写一条恒为真的判据比不写更糟 —— 它会让人以为那件事验过了。
 */

export const ITEMS = [
  // ——— M0 地基 ———
  {
    id: 'B0-1',
    milestone: 'M0',
    title: '门禁判定逻辑本身要被测试（防止恒为真的门禁）',
    verify: { kind: 'cmd', cmd: 'bash .scripts/gate-selftest.sh', cwd: 'wsl' },
    blocks: ['M0'],
  },
  {
    id: 'B0-2',
    milestone: 'M0',
    title: '一条命令能跑完全部门禁，且退出码可信',
    verify: { kind: 'manual', how: '在装有 node + cargo 的一侧跑 bash .scripts/gates.sh，看退出码' },
    blocks: ['M0'],
  },
  {
    id: 'B0-3',
    milestone: 'M0',
    title: '前端 lint 能跑，且已接进门禁',
    verify: { kind: 'cmd', cmd: 'bash .scripts/gates.sh --fast', cwd: 'wsl' },
    blocks: ['M0'],
  },
  {
    id: 'B0-4',
    milestone: 'M0',
    title: '根目录的本地临时文件不污染 git status',
    verify: { kind: 'cmd', cmd: 'git status --porcelain --untracked-files=all | grep -v "^ M" || true', cwd: 'host' },
    blocks: ['M0'],
  },
  {
    id: 'B0-5',
    milestone: 'M0',
    title: '三个空壳 crate 已删除（它们只在描述里声称有 SSRF 防护）',
    verify: { kind: 'absent', path: 'crates/quill-bridge' },
    blocks: [],
  },
  {
    id: 'B0-6',
    milestone: 'M0',
    title: 'git 历史里不再有指向 main 之外的分支',
    verify: { kind: 'manual', how: 'git branch -a —— 本项目不开分支，历史全在 main' },
    blocks: [],
  },

  // ——— M1 单人闭环 ———
  {
    id: 'B1-1',
    milestone: 'M1',
    title: '界面上不存在「点了必失败」的按钮',
    verify: { kind: 'manual', how: '在浏览器里逐页点一遍；每个入口要么真通，要么明确写成未接通并说明缺什么' },
    blocks: ['M1'],
  },
  {
    id: 'B1-1b',
    milestone: 'M1',
    title: '停用账号挡得住 QUILL_TOKENS（环境变量令牌回库核身份）',
    verify: { kind: 'test', name: 'bootstrap::tests::admin_token_provisions_an_owner_row' },
    blocks: [],
  },
  {
    id: 'B1-1c',
    milestone: 'M1',
    title: 'lint 接进门禁，不再是「只跑得起来但没人跑」',
    verify: { kind: 'cmd', cmd: 'bash .scripts/gate-selftest.sh', cwd: 'wsl' },
    blocks: [],
  },
  {
    id: 'B1-1d',
    milestone: 'M1',
    title: '前端审计的 44 条「为什么不修」已逐条给出理由',
    verify: { kind: 'manual', how: '读 BACKLOG.md 的 B1-1d，确认每条都有理由而不是「以后再说」' },
    blocks: [],
  },
  {
    id: 'B1-2',
    milestone: 'M1',
    title: '界面分得清「未注册 / 查无此人 / 未接通」三种状态',
    verify: { kind: 'test', name: 'dispatch_routes_are_registered_and_answer_in_chinese' },
    blocks: ['M1'],
  },
  {
    id: 'B1-3',
    milestone: 'M1',
    title: '会话改名返 405 时，界面如实提示而不是假成功',
    verify: { kind: 'manual', how: '点改名，确认界面说明「后端未开放该路由」' },
    blocks: [],
  },
  {
    id: 'B1-4',
    milestone: 'M1',
    title: '首轮浏览器实点验收跑完（备份收紧为 admin 之后需要重跑）',
    verify: { kind: 'manual', how: '全新一次性实例，不设 QUILL_TOKENS，逐页点一遍并贴实际结果' },
    blocks: ['M1'],
  },
  {
    id: 'B1-5',
    milestone: 'M1',
    title: 'M1 判据里的「派工」已划归 M2（否则 M1 永远达不成）',
    verify: { kind: 'manual', how: '确认 MILESTONES.md 的 M1 判据里没有派工' },
    blocks: [],
  },
  {
    id: 'B1-6',
    milestone: 'M1',
    title: '专家市场（列表 + 安装）接通',
    verify: { kind: 'manual', how: 'GET /api/experts/market 与 install 各点一遍' },
    blocks: [],
  },
  {
    id: 'B1-7',
    milestone: 'M1',
    title: '流式输出（SSE）接通，界面逐帧更新',
    verify: { kind: 'test', name: 'src/chat/ChatPage.test.tsx > 模型还在写的时候，界面就已经把那半句显示出来了' },
    blocks: [],
  },
  {
    id: 'B1-8',
    milestone: 'M1',
    title: '换会话时不残留上一会话的话',
    verify: { kind: 'test', name: 'src/chat/ChatPage.test.tsx > 切到另一个会话时，上一个会话的话不会留在屏幕上' },
    blocks: [],
  },
  {
    id: 'B1-9',
    milestone: 'M1',
    title: '备份三条路由只允许 admin（否则任何登录用户都能导出全部会话）',
    verify: { kind: 'test', name: 'a_logged_in_non_admin_cannot_reach_any_backup_route' },
    blocks: ['M5'],
  },

  // ——— M2 专家与专家团真执行 ———
  {
    id: 'B2-1',
    milestone: 'M2',
    title: '派工的 GET 与 POST 都不再丢弃 team_id',
    verify: { kind: 'test', name: 'listing_dispatch_of_a_missing_team_is_404_not_an_empty_200' },
    blocks: ['M2'],
  },
  {
    id: 'B2-2',
    milestone: 'M2',
    title: '成员执行器存在（子 agent 真被执行）',
    verify: { kind: 'absent', path: 'crates/quill-server/src/member_executor.rs' },
    blocks: ['M2'],
  },
  {
    id: 'B2-3',
    milestone: 'M2',
    title: '建团队静默多出的那个会话，是否要在侧栏隐藏（产品决定）',
    verify: { kind: 'manual', how: '建一个团队，看侧栏里那个 team_leader 会话，再定要不要藏' },
    blocks: [],
  },
  {
    id: 'B2-4',
    milestone: 'M2',
    title: 'teams 的限制列不再是摆设（max_dispatch / max_replan / max_ask_depth / guidelines）',
    verify: { kind: 'manual', how: 'grep crates/*/src 确认这四列仍零引用即未做；做了就换成 grep 判据' },
    blocks: ['M2'],
  },

  // ——— M3 上下文与成本 ———
  {
    id: 'B3-1',
    milestone: 'M3',
    title: '压缩阈值不再只存不读（真发生压缩，用量前后连续）',
    verify: { kind: 'manual', how: '超阈值后要真的发生压缩并能从压缩点续上；现在只有标注没有本体' },
    blocks: ['M3'],
  },
  {
    id: 'B3-1a',
    milestone: 'M3',
    title: '压缩阈值在界面上明写「暂未生效」（不允许出现不生效却不标注的开关）',
    verify: { kind: 'test', name: 'src/models/ModelsPage.test.tsx > 压缩阈值输入框 > 明写「暂未生效」：这个数存得下来，但没有任何代码读它' },
    blocks: [],
  },
  {
    id: 'B3-2',
    milestone: 'M3',
    title: 'token 口径与 goose 一致（缓存是输入的子集，总量按减法算）',
    verify: { kind: 'manual', how: 'B3-2 记录的悬而未决项：在没定上游之前不改' },
    blocks: [],
  },
  {
    id: 'B3-3',
    milestone: 'M3',
    title: '上下文图在加载态有渲染，不是空白',
    verify: { kind: 'manual', how: '断网/慢网下打开用量页的上下文图' },
    blocks: [],
  },

  // ——— M4 数据安全 ———
  {
    id: 'B4-1',
    milestone: 'M4',
    title: '备份导出真落盘，摘要与磁盘上的文件一致',
    verify: { kind: 'test', name: 'export_writes_a_real_backup_and_the_digest_matches_the_file_on_disk' },
    blocks: [],
  },
  {
    id: 'B4-1b',
    milestone: 'M4',
    title: '校验不是装饰：改一个字节必须失败并点名那个文件',
    verify: { kind: 'test', name: 'verify_fails_and_names_the_file_when_the_backup_is_tampered_with' },
    blocks: [],
  },
  {
    id: 'B4-1c',
    milestone: 'M4',
    title: '客户端不能指定落盘位置（../ 与盘符一律拒）',
    verify: { kind: 'test', name: 'api_backup::tests::hostile_names_are_refused_with_the_reason' },
    blocks: [],
  },
  {
    id: 'B4-2',
    milestone: 'M4',
    title: '在线升级机制存在（目前只有「升级前先备份」的守卫）',
    verify: { kind: 'manual', how: 'crates/quill-upgrade 只有约 107 行，真更新机制不存在；别按「只差路由」估工时' },
    blocks: ['M4'],
  },

  // ——— M5 多用户与多端 ———
  {
    id: 'B5-1',
    milestone: 'M5',
    title: '用户管理三层齐了（存储 / 服务 / 路由）',
    verify: { kind: 'test', name: 'unauthenticated_expert_requests_are_401_before_anything_else' },
    blocks: ['M5'],
  },
  {
    id: 'B5-1a',
    milestone: 'M5',
    title: '非 admin 令牌碰不到实例级配置',
    verify: { kind: 'test', name: 'admin_config_rejects_non_admin' },
    blocks: [],
  },
  {
    id: 'B5-2',
    milestone: 'M5',
    title: '/api/cron 有路由（自动化页配的是假后端）',
    verify: { kind: 'manual', how: 'curl 一下 /api/cron，现在是 404' },
    blocks: [],
  },
  {
    id: 'B5-3',
    milestone: 'M5',
    title: '不再上报没有真来源的字段（plugins / wiki_index / skill_count / tool_allowlist）',
    verify: { kind: 'manual', how: '按 BACKLOG B5-3 的逐列复核表再核一遍 crates/*/src' },
    blocks: [],
  },

  // ——— M6 上游对齐 ———
  {
    id: 'B6-1',
    milestone: 'M6',
    title: '.upstream-pin 与 UPSTREAM.md 不再自相矛盾',
    verify: { kind: 'cmd', cmd: 'node .upstream-check.mjs', cwd: 'wsl' },
    blocks: ['M6'],
  },
  {
    id: 'B6-2',
    milestone: 'M6',
    title: '自创机制已标出来（不包装成抄来的）',
    verify: { kind: 'cmd', cmd: 'node .scripts/provenance-selftest.mjs', cwd: 'wsl' },
    blocks: ['M6'],
  },
  {
    id: 'B6-3',
    milestone: 'M6',
    title: '每条上游引用逐行核过，且「没核」与「核过没问题」分开报',
    verify: { kind: 'cmd', cmd: 'node .provenance-check.mjs', cwd: 'wsl' },
    blocks: [],
  },
];

/** 里程碑的推进顺序。判据是人的判断，不是测量结果，所以留在这里。 */
export const MILESTONE_ORDER = ['M0', 'M1', 'M2', 'M3', 'M4', 'M5', 'M6'];

export const MILESTONE_NAMES = {
  M0: '可重复的地基',
  M1: '单人闭环（= MVP）',
  M2: '专家与专家团真执行',
  M3: '上下文与成本可信',
  M4: '数据安全',
  M5: '多用户与多端同步',
  M6: '与两个上游对齐',
};
