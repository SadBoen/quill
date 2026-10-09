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
    // 标 slow：gate-selftest.sh 要拿 7 个场景各跑一遍真正的 gates.sh
    // （注入 lint error、看它红；缺上游、看它报「未跑」…），
    // 所以它是分钟级命令。不标 slow 时 `node scripts/status.mjs --quick`
    // 实际要跑六七分钟 —— 一个叫「quick」的命令不该这样。
    verify: { kind: 'cmd', cmd: 'bash .scripts/gate-selftest.sh', cwd: 'wsl', slow: true },
    blocks: ['M0'],
  },
  {
    id: 'B0-2',
    milestone: 'M0',
    title: '一条命令能跑完全部门禁，且退出码可信',
    // 这条曾经是 manual，理由写的是「要人记得手动跑」。可它自己就写明了
    // 那条命令是 `bash .scripts/gates.sh` —— 判据里已经有一句可直接执行的话，
    // 把它标成人工，等于让机器能做而故意不做。
    //
    // 标 slow：它要编译并跑全工作区测试（分钟级）。--quick 必须跳过它，
    // 否则「秒级出结果」就成了谎话。skip 与「通过」在状态里分两栏显示。
    verify: { kind: 'cmd', cmd: 'bash .scripts/gates.sh', cwd: 'wsl', slow: true },
    blocks: ['M0'],
  },
  {
    id: 'B0-3',
    milestone: 'M0',
    title: '前端 lint 能跑，且已接进门禁',
    // 同 B0-2 标 slow：`--fast` 只跳过前端 build，**不跳过** Rust 全部测试与
    // vitest，所以它同样是分钟级命令。没标 slow 时 `node scripts/status.mjs --quick`
    // 会被它拖到五分钟以上 —— 「秒级出结果」就成了一句谎话。
    verify: { kind: 'cmd', cmd: 'bash .scripts/gates.sh --fast', cwd: 'wsl', slow: true },
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
    // 只看**本地**分支。origin/HEAD 与 origin/main 是远程跟踪引用，不是分支；
    // 把它们算进来会让判据恒红 —— 而恒红的判据等于没有判据，只会让所有人学会忽略它。
    // 变异验证：条件反过来（-ne 0）时确实变红。
    verify: { kind: 'cmd', cmd: 'test $(git branch --format="%(refname:short)" | grep -cvx main) -eq 0', cwd: 'wsl' },
    blocks: [],
  },

  // ——— M1 单人闭环 ———
  {
    id: 'B1-1',
    milestone: 'M1',
    title: '界面上不存在「点了必失败」的按钮',
    // 为什么只能人工：判据是「没有点了必失败的按钮」，而「失败」只在真点下去
    // 之后才存在。HTTP 层看不出界面有没有把 405 说成成功 —— 那正是假成功发生的地方。
    verify: { kind: 'manual', how: '为什么只能人工：失败只在真点下去之后才存在，HTTP 层看不出界面有没有把错误说成成功。在浏览器里逐页点一遍；每个入口要么真通，要么明确写成未接通并说明缺什么' },
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
    // 同 B0-1：命令一模一样，也就同样得标 slow。
    verify: { kind: 'cmd', cmd: 'bash .scripts/gate-selftest.sh', cwd: 'wsl', slow: true },
    blocks: [],
  },
  {
    id: 'B1-1d',
    milestone: 'M1',
    title: '前端审计的 44 条「为什么不修」已逐条给出理由',
    // 为什么只能人工：「理由写得够不够」是判断，不是事实。
    // 机器能数出有几条、能不能 grep 到「以后再说」，但「这条理由是否真的回答了
    // 为什么现在不修」没有可判定的形式。硬要自动化只会造出一个恒为真的判据。
    verify: { kind: 'manual', how: '为什么只能人工：「理由是否真的回答了为什么现在不修」是判断，没有可判定的形式。读 BACKLOG.md 的 B1-1d，确认每条都有理由而不是「以后再说」' },
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
    // 为什么只能人工：判据落在「界面如实提示」上，而 405 是完全正确的行为 ——
    // 后端确实没开放那条路由。要验的是界面有没有**把 405 说成成功**，
    // 那是渲染出来给人看的东西，只能人看。
    verify: { kind: 'manual', how: '为什么只能人工：要验的是界面有没有把 405 说成成功，那是渲染出来给人看的。点改名，确认界面说明「后端未开放该路由」' },
    blocks: [],
  },
  {
    id: 'B1-4',
    milestone: 'M1',
    title: '首轮浏览器实点验收跑完（备份收紧为 admin 之后需要重跑）',
    // 为什么只能人工：验收对象是「人点下去看到什么」，包括视觉层级、文案与
    // 意外状态。备份收紧为 admin 之后必须重跑，因为之前那轮验的是更宽松的权限，
    // 沿用旧结论等于用错的假设签字。
    verify: { kind: 'manual', how: '为什么只能人工：验收对象是「人点下去看到什么」，含视觉与意外状态。全新一次性实例，不设 QUILL_TOKENS，逐页点一遍并贴实际结果（备份收紧为 admin 之后必须重跑，旧结论验的是更宽松的权限）' },
    blocks: ['M1'],
  },
  {
    id: 'B1-5',
    milestone: 'M1',
    title: 'M1 判据里的「派工」已划归 M2（否则 M1 永远达不成）',
    // 为什么只能人工：M1 的边界是散文里的一节，不是可解析的结构。
    // 用 grep 卡「M1 段落里不许出现派工」很容易在边界识别失败时**静默通过** ——
    // 那比不自动化更坏：它会让人以为这条已经被机器守着。
    verify: { kind: 'manual', how: '为什么只能人工：M1 的边界是散文里的一节，grep 卡边界失败时会静默通过，比不自动化更坏。确认 MILESTONES.md 的 M1 判据里没有派工' },
    blocks: [],
  },
  {
    id: 'B1-6',
    milestone: 'M1',
    title: '专家市场（列表 + 安装）接通',
    // 原来是「需人工」，理由是「现有 api_expert_market 测试验的是 slug 长度与
    // 列约束等边界，绑上来会一直绿而市场页仍然死」—— 这个顾虑是对的，
    // 所以**不是**去绑那类边界测试。
    //
    // 2026-10-08 在浏览器里实点发现：这条早就做完了，而且前端已有测试钉着
    // 「安装后逐条显示技能状态、失败原因、人格来源与停用提示」——
    // 正是实点看到的那件事（点「安装为我的专家」→ 已安装 + 6 个技能逐个「已装入」
    // + 「技能一律停用」的提示）。所以绑的是这条，而不是边界测试。
    verify: { kind: 'test', name: 'src/experts/MarketTab.test.tsx > 安装后逐条显示三种技能状态、失败原因、人格来源与停用提示' },
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
    // 2026-10-08 修正过一次：原来绑的是 `absent crates/…/member_executor.rs` ——
    // **方向写反了**，文件不存在反而判「已验证」。当时改成 grep 具体结构，
    // 因为那会儿生产侧确实没有任何具体实现（只有 quill-agent 的泛型
    // SharedExecutor 包装 + testkit 的 mock）。
    //
    // 2026-10-08 同日落地：`crates/quill-server/src/member_executor.rs` 的
    // `ProviderMemberExecutor` 真的调模型，并接到
    // `POST /api/teams/{id}/dispatch/run`（见 api_dispatch::run）。
    // 所以判据从「结构存在」升到**行为**：绑那条端到端测试 ——
    // 它验的是「每个成员真被调了一次模型，产出进了 results」。
    verify: {
      kind: 'test',
      name: 'running_a_round_really_calls_the_model_and_returns_the_output',
    },
    blocks: ['M2'],
  },
  {
    id: 'B2-3',
    milestone: 'M2',
    title: '建团队静默多出的那个会话，不在侧栏露出来',
    // 产品决定已做（2026-10-07，BACKLOG 有完整理由）：那条会话是派工记账的
    // 落点，删不掉也不该删，但它属于实现细节、不该让用户点进一个空会话。
    //
    // 判据钉的是**过滤真的生效**，而不是「界面上看不见」——后者只能靠眼睛，
    // 而这条 API 是团队页也要用的全量入口，把侧栏的诉求钉在它身上会连累团队页。
    // 细节（为什么是逗号分隔、为什么不用重复键、为什么过滤放后端）见
    // crates/quill-server/src/api_chat.rs 的 SessionListQuery 文档注释。
    verify: { kind: 'cmd', cmd: 'cargo test -p quill-server --test session_kind_filter_http', cwd: 'wsl' },
    blocks: [],
  },
  {
    id: 'B2-4',
    milestone: 'M2',
    title: 'teams 的限制列不再是摆设（max_dispatch / max_replan / max_ask_depth / guidelines）',
    // 原来写的是 manual，how 里还留着一句「做了就换成 grep 判据」。
    // 那句话本身就是这条待办没被收尾的证据 —— 判据已经想清楚了却一直没接上。
    // 现在接上了：grep 命中即视为「不再是摆设」。
    //
    // 2026-10-07 实测 exit=1（crates 下零引用，确实还是摆设），
    // 所以它现在会如实报「未通过」而不是躲在「需人工」里。
    // 变异验证：条件反过来时 exit=0。
    verify: { kind: 'cmd', cmd: 'grep -rqE "max_dispatch|max_replan|max_ask_depth|guidelines" crates/quill-server/src crates/quill-store/src', cwd: 'wsl' },
    blocks: ['M2'],
  },
  {
    id: 'B2-5',
    milestone: 'M2',
    title: '成员的任务与产出进各自的会话（Q025）',
    // 「每子 agent 独立会话」本身对齐 goose（`agents/subagent_handler.rs:178-190`：
    // 独立 session_id 经 SessionManager 落盘）；quill 自己的是**存法**（全部会话
    // 在同一张 sessions 表里靠 kind 分家，所以成员会话要靠 exclude_kind 才不混进
    // 侧栏），对齐与代价记在 docs/UPSTREAM-DIVERGENCES.md 的 B1。
    //
    // 判据绑端到端用例（不是「表里有没有行」这种结构检查）：
    // run 完一轮后经 GET /api/sessions/{id}/messages 读回两条，
    // 且任务正文与成员实际收到的 prompt 逐字相同 —— 写回的若是事后复述，
    // 这条会红。
    verify: {
      kind: 'test',
      name: 'a_round_writes_task_and_output_into_the_member_session',
    },
    blocks: ['M2'],
  },

  // ——— M3 上下文与成本 ———
  {
    id: 'B3-1',
    milestone: 'M3',
    title: '压缩阈值不再只存不读（真发生压缩，用量前后连续）',
    // 2026-10-09（Q109）：Q018 把压缩接进对话路径后，这条不再「只能人工」——
    // 两半都各有测试钉住：
    //   · 真发生压缩 + 从压缩点续上：`crates/quill-server/tests/chat_compaction_http.rs`
    //     的 `a_history_over_the_threshold_is_really_compacted`（端到端，走 HTTP）
    //   · 用量前后连续：`crates/quill-core/src/compaction.rs`
    //     的 `billable_usage_is_continuous_across_compaction`
    // 这里绑端到端那条（更强的那个）；用量那半写在上面，免得只剩一半被机器看着。
    verify: { kind: 'test', name: 'a_history_over_the_threshold_is_really_compacted' },
    blocks: ['M3'],
  },
  {
    id: 'B3-1a',
    milestone: 'M3',
    // 2026-10-09 修正（Q109）：Q018 把压缩接线落地后，界面标注从「暂未生效」
    // 改成了「已生效」，测试名跟着改了 —— 而这里还绑着旧名，于是这条判据
    // **永久落在 BROKEN**（`node scripts/status.mjs --self-check` 在 WSL 下
    // 实测报「1 处有问题」）。判据绑错名字比判据没通过更糟：它的真实状态是未知。
    title: '压缩阈值在界面上明写「已生效」与估算口径（不允许出现不生效却不标注的开关）',
    verify: { kind: 'test', name: 'src/models/ModelsPage.test.tsx > 压缩阈值输入框 > 明写「已生效」与口径：这个数真的会被读，数字是估算值' },
    blocks: [],
  },
  {
    id: 'B3-2',
    milestone: 'M3',
    title: 'token 口径与 goose 一致（缓存是输入的子集，总量按减法算）',
    // 为什么只能人工：口径本身还没定。缓存算不算输入的子集、总量按加还是按减，
    // 取决于上游怎么算；在没对齐之前，任何「验证」都是在给一个未定的口径背书。
    verify: { kind: 'manual', how: '为什么只能人工：口径尚未对齐上游，此刻任何验证都是给未定的口径背书。BACKLOG B3-2 记录的悬而未决项：在没定上游之前不改' },
    blocks: [],
  },
  {
    id: 'B3-3',
    milestone: 'M3',
    title: '上下文图在加载态有渲染，不是空白',
    // 为什么只能人工：现有 ContextWindowChart 测试验的是「没有实测值时明说没量过」
    // 这类**空态**，不是「加载态有渲染」。加载态要的是真把网络拖慢才看得见。
    verify: { kind: 'manual', how: '为什么只能人工：现有测试验的是空态而非加载态，加载态要真把网络拖慢才看得见。断网/慢网下打开用量页的上下文图' },
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
    id: 'B4-3',
    milestone: 'M4',
    // 2026-10-09 新增（Q094 / Q058）：资料库页面从「只能读」变成「能读写」，
    // 而写入这类东西的判据必须是端到端的 —— 光有路由不算数。
    title: '资料库页面可手写增删改，且带乐观并发（过期版本不许覆盖）',
    // 这一条绑一个测试文件，它一次钉住五件事：新建/覆盖/删除**真落盘**并重建索引
    // 与变更日志、过期版本 409 **且不动盘**、重复新建 409、写不进去的内容在动盘前
    // 被 400 挡下、越界路径进不来。任何一条坏掉，这个命令都会非 0。
    verify: {
      kind: 'cmd',
      cmd: 'cargo test -p quill-server --test wiki_write_http',
      cwd: 'wsl',
    },
    blocks: [],
  },
  {
    id: 'B4-2',
    milestone: 'M4',
    title: '在线升级机制存在（目前只有「升级前先备份」的守卫）',
    // 为什么只能人工：判据是「真更新机制存在」，而现状是 crates/quill-upgrade
    // 只有约 107 行、只有升级前先备份的守卫。可以写「行数 > N」这种判据，
    // 但 N 是拍的 —— 它会在有人写了 200 行占位代码时变绿，那比恒红更糟。
    verify: { kind: 'manual', how: '为什么只能人工：「机制存在」无法用行数阈值表达——阈值是拍的，会被占位代码骗过。crates/quill-upgrade 只有约 107 行，真更新机制不存在；别按「只差路由」估工时' },
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
    // 为什么只能人工：1403 条测试名里 grep 不到任何与 cron 相关的用例 ——
    // 路由根本不存在，自然也没有测试可以绑。自动化页配的是假后端这件事
    // 只有真发一次请求才知道；在路由建起来之前，任何判据都只能绑一个 404。
    verify: { kind: 'manual', how: '为什么只能人工：1403 条测试名里没有任何 cron 相关用例——路由不存在就没有测试可绑。curl 一下 /api/cron，现在是 404' },
    blocks: [],
  },
  {
    id: 'B5-3',
    milestone: 'M5',
    // 2026-10-09 重绑（Q103/Q104）：这条原来叫「不再上报没有真来源的字段
    // （plugins / wiki_index / skill_count / tool_allowlist）」，命令是
    // `! grep -rqE "wiki_index|skill_count|tool_allowlist" …`。2026-10-08 的 Q044
    // 逐条核过之后，那句话的前提就不成立了（那几个名字要么有真来源、要么压根没在上报），
    // 于是那条命令**永远不可能通过** —— 一条恒红的判据等于真实状态未知。
    // 真正还在的洞是三处**死结构**，其中两处现在已清掉：`experts.skill_count` 由迁移
    // 0012 删列、`plugins`/`wiki_index` 两张死表由 0013 删除。剩下 `skills.tool_allowlist`
    // 存而不用（要不要真消费是 Q102 的产品决定，与「上报」无关）。
    title: '死结构已清掉（experts.skill_count 与 plugins / wiki_index 两张死表）',
    // 判据绑在那两条迁移的测试上：它们验的是「列真的没了 / 表真的没了」，
    // 而不是「grep 不到某个名字」（后者会被注释、夹具、迁移文件本身命中）。
    verify: { kind: 'cmd', cmd: 'cargo test -p quill-store migration_001', cwd: 'wsl' },
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
  {
    id: 'B6-4',
    milestone: 'M6',
    title: 'vendor/openoctopus-frontend 的来源查到了：它不是 Octop',
    // 2026-10-08 定位。此前只知道「来源没记」，真去查才发现问题更大：
    // 那棵树是 `github.com/Zpoteiti/OpenOctopus`（MIT, Copyright 2026 Yucheng Zou），
    // **与 Octop 无亲缘关系的另一个项目**，只贡献 index.css。
    // 真 Octop 是 `github.com/TencentCloud/Octop`（本地检出 .octop-ref/octop/）。
    //
    // 事故：照那棵树写了几百行通道实现，而它里面没有微信、没有 personalization。
    // 见 BACKLOG B6-4 —— 那里记了「文档里的免责声明会反向授权」这件事本身。
    //
    // 判据是自动的：门禁 .octop-baseline-check.mjs 全仓扫一遍，
    // 任何把它当 Octop 功能参考的引用都报红。
    // 判据是自动的：门禁 .octop-baseline-check.mjs 全仓扫一遍，
    // 任何把它当 Octop 功能参考的引用都报红。
    //
    // 2026-10-08 修正：原来写的是 `kind: 'script'` —— 而 status.mjs 的 decide()
    // 只认 test/cmd/absent/manual，于是这条一直被判成「判据本身坏了」（真实状态未知）。
    // 现在改成 cmd，与它的实质（跑一条脚本看退出码）一致。
    verify: { kind: 'cmd', cmd: 'node .octop-baseline-check.mjs', cwd: 'wsl' },
    blocks: [],
  },
  {
    id: 'B6-5',
    milestone: 'M6',
    title: '通道与个性化页面按 TencentCloud/Octop 重做',
    // 用户原话：「通道也搞起来吧，连接一个微信就可以了」+「通道 + 个性化页面一起做」。
    // 起因是 B6-4 —— 参考基准指错了地方。
    //
    // 2026-10-08 已完成：迁移 0009_channels.sql、channels/{store,weixin}.rs、
    // api_channels.rs（REST + 微信扫码三步 + 长轮询后台任务）、
    // ui/web/src/channels/（页面 + 8 条判据）。
    // 判据：channels_http 14 条 + store 5 条 + weixin 11 条 + api_channels 11 条
    //      + 前端 8 条；变异验证 6 个变异全被抓住。
    //
    // 仍缺两件，都已如实标在界面上：
    // 1. **没做过真机扫码**。微信那套三步与长轮询全是照公开协议写的，
    //    判据只覆盖了请求体校验与凭据不外泄，真跑一次才算数。
    // 2. **MBTI**。要四维光谱表 + 28 题测评结果的存储，动数据结构。
    //
    // 顺带核清一件先前记错的事：「八张人格卡」不存在。Octop 那个页面的
    // 页签只有七个（skills/subagents/tools/plugins/mbti/memory/channels），
    // 每一签都是复用别处的面板；IDENTITY.md / SOUL.md / HEARTBEAT.md 那些
    // 是专家库里的文件，不是页签，agent_files.py 也不读它们。
    // 而 quill 的人格注入本来就是通的（experts.instructions 一列 →
    // resolve_persona → system prompt），library/*/SOUL.md 只是移植史料 ——
    // 把它们接进 prompt 反而会让用户改史料以为改了行为，所以没做。
    //
    // 已完成：ui/web/src/personalization/ 聚合页（6 条判据 + 变异验证），
    // 两处「不许粉饰」：MBTI 明说没做、子智能体明说派工不执行（B2-2）。
    // —— 其中「MBTI 明说没做」这条已在 B6-6 落地，个性化页那张卡换成真页面。
    verify: {
      kind: 'manual',
      how: '用微信扫一次真账号，能收到消息并收到回复；界面点一遍个性化页各入口',
    },
    blocks: [],
  },
  {
    id: 'B6-6',
    milestone: 'M6',
    title: '人格（MBTI）：28 题测评 + 四维光谱 + 应用到某个专家',
    // 起因是 B6-5 里那条「MBTI 明说没做」。用户要求把 Octop 个性化页面的
    // 功能都学过来，MBTI 是其中唯一一块真的要动数据结构的。
    //
    // 核到的上游：mbti_profiles.py（16 型 596 行）、mbti.py 里的 _QUESTIONS
    // （28 题）、_score_answers（:617-673）、MBTISelector.tsx / MBTITest.tsx。
    //
    // 2026-10-08 已完成：mbti/{profiles,questions,score,store,apply}.rs、
    // api_mbti.rs、迁移 0010_mbti.sql、ui/web/src/mbti/（页面 + 判据）。
    // 判据：lib 21 条 + HTTP 13 条 + 前端 16 条。
    //
    // 三处是「我们的选择」，不是照抄：
    // 1. 不写 SOUL.md（Octop 的 mbti.py:58），本项目的人格正文是
    //    experts.instructions；
    // 2. /api/mbti/apply 强制带 expert_id —— 人格挂在专家上，
    //    一个用户有多个专家，没有「当前智能体」这个说得清的默认目标；
    // 3. 存历史（每人最近 20 条 + 原始作答），Octop 只有一个 persona_mbti 字段。
    // 2026-10-08 修正：原来写的是 `kind: 'auto'`（decide() 不认，恒判「判据坏了」）。
    // 它的实质是跑几组测试，改成 cmd。
    verify: {
      kind: 'cmd',
      cmd: 'cargo test -p quill-server --lib mbti && cargo test -p quill-server --test mbti_http',
      cwd: 'wsl',
      slow: true,
    },
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
