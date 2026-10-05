# 问题记录

每条按这个格式记。**只修不留痕等于没修** —— 下次谁都不知道这里踩过什么。

判定边界见 `README.md`：**答不上来不算 bug**（本机 4B 模型），
链路断、状态说谎、错误被吞、隔离失效才算。

---

## 格式

```
## ISSUE-nnn · 一句话标题

- **发现于**：任务 id / 手工浏览 / 门禁
- **现象**：用户看到什么。写用户视角，不写代码视角。
- **严重度**：阻断 / 严重 / 一般 / 轻微
- **根因**：为什么会这样。指到文件与行。
- **复现**：一步步怎么触发。
- **修复**：改了什么，为什么这么改（有没有更小的改法，为什么不用）。
- **回归**：钉住它的测试叫什么；全量门禁多少 passed / 0 failed。
- **状态**：待修 / 已修待回归 / 已修已回归 / 挂起（附挂起理由）
```

---

## ISSUE-000 · 迁移在多连接池上必炸（已修）

- **发现于**：接入 MCP 配置存储后跑全量门禁
- **现象**：`quill doctor` 退出码 2，报
  `there is already another table or index with this name: mcp_servers`；
  6 个 CLI 用例全红。**注意这是真实服务才有的现象**，测试里一直绿。
- **严重度**：阻断
- **根因**：`quill_store::run_migration` 逐条 `sqlx::query(s).execute(pool)`，
  而 `SqlitePool` 每条 query 各自取连接。0007 开头的
  `PRAGMA foreign_keys = OFF` 落在连接 A，紧接着的 `DROP TABLE` / `ALTER TABLE`
  却在连接 B 上照旧开着外键。
  **为什么测试没抓到**：`quill-store` 的迁移测试用 `in_memory()`，
  那条池 `max_connections(1)`，单连接下 PRAGMA 永远生效；真实服务是
  `configure_pool(path, 5)`，五连接，问题必现。
- **复现**：`configure_pool(path, 5)` 建池，握住一条连接不放，再
  `migrate(&pool)`。
- **修复**：`run_migration` 固定用一条连接并包在一个事务里。顺带失败时
  错误带「第 N/M 条语句 + 语句片段」。
  **为什么不逐条提交**：一条迁移要不就全成要不就全不成；逐条提交会留下
  半张表，而台账还没写，下次启动当没跑过再来一遍。
- **回归**：`all_migrations_apply_through_a_multi_connection_file_pool`、
  `a_failing_migration_leaves_nothing_behind`、
  `a_failing_statement_says_which_one_it_was`
- **状态**：已修已回归

## ISSUE-001 · 指纹长度会让每一行写入被数据库拒掉（已修）

- **发现于**：写 `mcp_repo` 时自查 schema
- **现象**：每一行 INSERT 都被拒，而错误只说
  `CHECK constraint failed`，不告诉你是哪一列。
- **严重度**：阻断
- **根因**：`mcp_servers.asset_hash` 与 `skills.content_hash` 的 CHECK
  都是 `length = 32`，我截 SHA-256 到 16 字节。**编译过、单测也过**，
  只有真写库才炸。
- **修复**：复用 `db::digest32`（本来就返回 32 字节，`experts` 在用）。
  顺带把分段改成「个数 + 每项长度前缀」——`[""]` 与 `[]`、
  `["a\u{1}b"]` 与 `["a","b"]` 这两组是被自己的单测抓出来的真碰撞。
  另外 `env` 与 `headers` 拆成两个区段算，合成一张 map 会让同名字段互相覆盖。
- **回归**：`the_asset_hash_is_the_length_the_check_constraint_demands`、
  `field_boundaries_cannot_be_forged_by_shifting_a_delimiter`、
  `env_and_headers_are_hashed_as_two_separate_sections`、
  `the_capability_three_states_are_three_different_fingerprints`
- **状态**：已修已回归

## ISSUE-002 · 「保存 → 重新编辑」这一圈是死的（已修）

- **发现于**：浏览器实测设备页
- **现象**：保存后想编辑这条配置，页面直接拦下提交，浏览器给一个
  没有来由的红框。
- **严重度**：严重
- **根因**：前端 `<input name="name">` 的 `pattern` 是
  `[a-z][a-z0-9_]{0,31}`（只允许下划线），服务端把 `_` 归一成 `-` 并回填
  `company-search`。回填的值过不了页面自己的校验。旧写法还要求首字符是
  字母，而服务端允许数字开头。
- **复现**：设备页新增一条名为 `Company Search` 的服务 → 保存 → 点编辑 →
  改任何东西 → 点保存 → 被拦。
- **修复**：`MCP_NAME_PATTERN` 改成 `[a-z0-9]([a-z0-9-]{0,62}[a-z0-9])?`。
  在 `mcp_repo.rs` 里加了同源表子，方向是**「服务端产出 ⊆ 前端能收」**——
  反过来不成立：前端比服务端严一点是提前拦下，严过头才是 bug。
- **回归**：`every_normalized_name_passes_the_frontend_pattern`（Rust）、
  `mcpConfig.test.ts` 的 4 条
- **状态**：已修已回归

## ISSUE-003 · 界面把已经接通的路由说成 501（已修）

- **发现于**：接完 MCP/SKILL 后回头查文案
- **现象**：对话页工具坞与专家库生成面板都把
  `GET /api/extensions/{mcp,skills}` 标成「未接通 · 501」，而这两条路由
  实际返回 200。
- **严重度**：严重（对着用户说谎）
- **根因**：状态是硬编码在两张前端表里的，后端接通了没人回来改。
- **修复**：抽到 `ui/web/src/capabilityGaps.ts` 单一数据源，两处共用；
  文案分 `partial`（存储层已通、执行层没接）与 `not-implemented`
  （路由压根不存在）两类。
- **回归**：`capabilityGaps.test.ts`、`LibraryTab.test.tsx` 新增那条
- **状态**：已修已回归

## ISSUE-004 · 删不存在的 MCP 服务仍报「已删除」（已修）

- **现象**：`DELETE /api/extensions/mcp/{name}` 对不存在的名字也返回
  `"deleted": true`。
- **严重度**：一般
- **根因**：实现走「读全量 → 去掉目标 → 写回」，无论目标在不在都写 `true`。
- **修复**：按过滤前的列表判断目标是否真的存在。
- **回归**：`delete_reports_honestly_when_there_was_nothing_to_delete`
- **状态**：已修已回归

## ISSUE-005 · 设备页文案说「服务端会回 501」（已修）

- **现象**：设备页顶部写着「quill 尚未实现该路由的实现体时，服务端会直接回
  501」。路由早就实现了。
- **严重度**：一般
- **根因**：同一批硬编码文案。
- **修复**：改成读服务端返回的 `connected` 与 `note`，原样显示；
  组件里**不许**出现任何由 `servers.length` 推断出来的「已连接」字样。
- **回归**：`saving_mcp_never_claims_to_be_connected`（HTTP 层）、
  `McpLinkStatus` 组件
- **状态**：已修已回归

## ISSUE-006 · SKILL 名带下划线会被数据库拒掉，「顶掉内置工具」这条路径不可达

- **发现于**：把 SKILL 挂进 `ToolRegistry` 时写回归测试
- **现象**：想测「一个叫 `list_experts` 的 SKILL 会顶掉内置工具」这条风险，
  在测试里绕过 HTTP 入口直接往 `skills` 插 `name='list_experts'`，插入失败：
  `CHECK constraint failed: name GLOB '[a-z0-9]*' AND name NOT GLOB '*[^a-z0-9-]*' AND name NOT GLOB '*..*'`。
- **严重度**：轻微（不是产品 bug，但影响守卫该怎么写）
- **根因**：0001 迁移里 `skills.name` 的 CHECK 只允许 `[a-z0-9-]`，**下划线不在内**；
  `normalize_name` 也把 `_` 换成 `-`。两边一致地禁止「SKILL 与内置工具同名」，
  所以 `tools.rs` 里那个同名守卫在今天的 schema 下是一条**跑不到**的分支。
- **复现**：往 `skills` 插任意 `name='list_experts'` 的行（换 user_id 也一样）。
- **修复**：把两条否决路径（正文为空 / 与已有工具同名）从 `with_skills` 里就地判断
  提成纯函数 `tools::veto`，直接对它单测，不再依赖「能不能造出脏数据」。
  **为什么保留这个守卫**：MCP 工具名恰恰常用连字符（`read-file`），b 线铺完 rmcp
  之后它就有用了 —— 但那时必须有一条测试能走到它。
  集成测试改成断言两条**可达**的不变量：`a_dash_named_skill_coexists_with_the_underscore_named_builtin`、
  `the_schema_itself_forbids_a_skill_from_taking_a_builtin_tools_underscored_name`。
- **回归**：`a_skill_with_no_body_on_disk_is_vetoed_rather_than_registered_empty`、
  `a_skill_may_not_replace_a_tool_that_already_has_that_name`（单测）；
  上面两条集成测试；全量 907 passed / 0 failed
- **状态**：已修已回归

## ISSUE-007 · SKILL 正文整段进工具描述，每轮请求都要为它付 token

- **发现于**：接 `with_skills` 时读 `as_tool_spec`
- **现象**：`as_tool_spec` 把整段正文放进 `ToolSpec::description`，而工具描述是
  **每一轮请求**都带在 `tools` 字段里的。`MAX_SKILL_CHARS` 是 20000，
  装 3 个长 SKILL 就是每轮多约 60000 字符，**哪怕这一轮一个技能都没用到**。
- **严重度**：严重（与设计理由直接矛盾）
- **根因**：`skills_repo.rs` 里 `ToolSpec::new(&r.name, content)` 把正文当描述。
  而这套设计的理由写的是「用不到就不占常驻上下文」—— 实际做到的是
  「用不到也每次都出现，只是省掉了正文的返场」。理由与实现不符。
- **复现**：装一个 20000 字符的 SKILL，看任意一次 chat 请求体里 `tools` 的长度。
- **修复**：**未修**。`as_tool_spec` 的语义是既有决定且已被
  `a_skill_becomes_a_tool_with_a_requiring_task_argument` 钉着（断言正文进 description），
  这一轮的任务是「挂进去」，顺手改语义就是两件事混在一起。
  候选改法：描述只放 `row.description`，正文由 handler 在被调用时返回 ——
  这才是「用不到完全不出现」的本来意思。代价是正文会过 `MAX_RESULT_CHARS`（4000）
  截断，要一并决定上限策略。**两个以上 SKILL 会有同样问题，不止一个。**
- **回归**：无（尚未修）。修的时候要给一条「装 3 个长 SKILL 时 `tools` 长度不随正文增长」
  的测试。
- **状态**：待修

## ISSUE-008 · `tool_allowlist` 只存不用

- **发现于**：接 `with_skills` 时查 `SkillRow` 的字段
- **现象**：`skills.tool_allowlist`（SKILL 允许调用哪些工具）有列、有读写、
  有 `to_json` 输出，但**没有任何一行代码读它来做决定**。一个 SKILL 声称
  「我只能用 `list_experts`」时，这个约束当前不生效。
- **严重度**：轻微（现在）/ 严重（b 线之后）
- **根因**：上一轮做存储层时把它当纯数据存了下来，执行层没接，所以没有消费方。
- **复现**：存一条带 `tool_allowlist` 的 SKILL，然后在 `tools.rs` 里搜该字段 —— 0 处引用。
- **修复**：**未修**。SKILL 目前只是「一套方法」不真的调工具，所以暂时不构成安全问题；
  但 b 线把 MCP 工具接进来之后，这就是一个**用户以为有限制、实际没有**的字段 ——
  与本项目「不许显示成接上了」的原则冲突。**b 线开始前必须处理。**
- **2026-10-06 补充（读执行路径后的新发现，原文的修复方向可能是错的）**：
  这条**没有消费方，不只是「忘了接」**。SKILL 在本项目里是
  `as_tool_spec` 把正文塞进工具描述 + `skill_handler` 只回一句「已加载这套方法」，
  也就是说 **SKILL 根本不执行工具调用**。而 `api_chat` 的工具循环是扁平的
  （`while !reply.tool_calls.is_empty()`，上限 `MAX_TOOL_ROUNDS`），
  **没有任何「当前是哪个技能在驱动这一轮」的归因**。所以「这个技能只能调 A、B」
  在今天的架构里**没有可以落地的执行点**。
  据此建议：要么先给工具循环补一层 per-turn 作用域再谈执行（b 线之后的事），
  要么就承认这个字段是**存着但不生效**，在 `GET /api/extensions/skills` 上如实报
  「`tool_allowlist` 尚未生效」—— 照 ISSUE-014 那条「别让界面替后端说谎」的口径处理。
  **不要**在没想清楚语义之前把它悄悄改成会话级工具表裁剪：那会把「这个技能只能用 X」
  变成「只要装了这个技能，整个会话都只能用 X」，是另一种骗人。
- **回归**：无（尚未修）。
- **状态**：待修

---

## ISSUE-009 · 界面把「已挂进对话工具表」的技能包说成「尚未挂进」

**状态**：已修（2026-10-06）

- **现象**：`GET /api/extensions/skills` 只报 `enabled` 与 `content_missing`，
  界面上「技能包」那一条写的是「能存能读；尚未挂进对话的工具表」
  （`ui/web/src/capabilityGaps.ts`）。而 `ToolRegistry::with_skills` 上一轮已经把
  启用的 SKILL 挂进对话的工具表了 —— 界面在把**已经能用**的东西说成不能用。
  中英文各一份文案（`i18n/resources.ts`、`ChatPage.tsx`、`LibraryTab.tsx`）
  以及 `libCapabilityNote` 全都停在旧状态。
- **根因**：存储层、执行层、界面三层各自演进，没有一个测试钉住
  「界面说的」与「后端做的」必须一致。`with_skills` 决定挂不挂，
  `list_skills` 决定显示什么，两边各判一次 —— 必然漂。
- **复现**：
  1. 装一个 `enabled: true` 且正文在磁盘上的 SKILL。
  2. `GET /api/extensions/skills` —— 响应里没有任何字段说「模型看不看得见」。
  3. 界面上仍显示「尚未挂进对话的工具表」。
- **修复**：
  1. 抽出 `tools::skill_visibility(existing, row, body) -> SkillVisibility`
     （`Visible` / `Disabled` / `NotMounted(原因)`）。`with_skills` 与
     `list_skills` 调**同一个**函数 —— 两处各判一次是这类 bug 的根源。
  2. `list_skills` 每条加 `model_can_see`，挂不上时加 `not_mounted_reason`
     （停用不算故障，**不带** reason）。`content_missing` 保留不动。
  3. 界面文案改成实话：技能包「已挂进对话工具表（每条带 model_can_see）；
     MCP 来的工具还没有」。`partial` 保留 —— 确实还缺 MCP 那一半。
  4. `capabilityGaps.ts` 顶部注释同步改掉「存储层已通，执行层没接」这个已过时的定义。
- **回归**：
  - `extensions_http` 新增 4 条，其中
    `the_api_says_model_can_see_a_skill_exactly_when_it_reaches_the_tool_table`
    是支点：它把界面报的布尔值与**真实工具表**逐条比对，任一边漂了都会红。
  - 反向验证：把 `model_can_see` 写死成 `true` 后 3 条测试变红（已复现并撤回）。
  - `tools` 单测 2 条（Disabled 不许伪装成 NotMounted、Visible/NotMounted 判定）。
  - 前端 `capabilityGaps.test.ts` 新增 2 条；`LibraryTab.test.tsx` 原来有一条
    断言写死了旧文案「部分接通 · 能存能读」，本次一并改成新文案并加反向断言
    （界面不许再出现「尚未挂进 / 还调不到」）。
  - 全量：`cargo test --workspace` 913 passed / 0 failed、`ui/web` 62 passed、
    `typecheck` 干净、`i18n-check` 0 问题、`library-check` 334/334。

---

## ISSUE-010 · 草稿卡片和已保存的配置长得一模一样，界面在骗人

**状态**：待修（2026-10-06 用浏览器真点发现）

- **现象**：在「设备 → 扩展」页填完表单点「加入草稿」，草稿立刻以**完全一样的卡片**
  出现在已保存配置列表里：同样的图标、同样的传输方式、同样的能力标签、
  同样挂着可用的「编辑」和「删除」按钮。**没有任何标记说明这一行还没落库。**
  用户以为存好了就关掉标签页，这条配置就永远丢了。
- **严重度**：严重（正是本项目红线里的「界面不许显示成接上了」）
- **复现**（本次真跑，非推演）：
  1. 服务端此时有 2 条：`company-search`、`filesystem`。
  2. 浏览器填 `browser-e2e-probe` / stdio / `python3 -m calculator_mcp`，点「加入草稿」。
  3. `curl /api/extensions/mcp` —— **仍然只有 2 条**，服务端根本没这条。
  4. 界面上却渲染出了第 3 张卡片，结构与前两张逐项相同。
  5. 点「保存 MCP 配置」后，curl 才返回 3 条。**保存这一步是真的。**
  6. 另测：对**未保存**的草稿点「删除」，抓网络面板 —— **一个请求都没发**，
     只是本地移除。所以删除按钮在草稿上的语义和它的文案（「删除 X」）也对不上。
- **根因**：草稿是前端本地状态，和服务端返回的 `servers` 拼进同一个数组渲染，
  渲染层分不清两者。顶部那句「已保存配置，尚未连接验证」说的是连接状态，
  **没有一句在说这行本身还没存**。
- **修复方向**（未定，需先定产品语义）：
  1. 草稿卡片必须有视觉与文案上的「未保存」标记，且它的「删除」要改成
     「丢弃草稿」，不能沿用已保存行的 aria-label。
  2. 或者更保守：草稿不进正式列表，另起一块「待保存（N）」区域，
     顶部写清「这 N 条还没写入服务器」。
  3. 无论选哪个，都要有一条测试钉住：**界面渲染出的卡片数 == 服务端返回的条数**，
     草稿单独计数，不许混算。
- **回归**：无（尚未修）。

---

## ISSUE-011 · 「能力全开」被当成「一个能力都没列」，工具数永远是 0

**状态**：已修（2026-10-06，b 线铺 MCP 协议层时由端到端测试抓到）

- **现象**：一台 stdio MCP 服务器**真的握手成功了**（界面显示已连上、协议版本
  与实现名都对），但工具数是 **0**。用户看到的是「连上了，可是没工具」，
  于是去检查自己装的东西、去看服务器文档 —— 而服务器明明报了三个工具。
- **严重度**：严重
- **根因**：`mcp_client::tools_capability_on` 写成
  `row.enabled_capabilities.iter().any(|c| c == "tools")`，
  把三态压成了「列表里有没有 tools」。`mcp_repo` 定义的语义是三态：
  `None` = 全禁、**`Some(vec![])` = 全开**、`Some(v)` = 精确列举。
  空数组的 `any` 恒为 `false`，于是**全开被判成全禁**。
  而默认配置（`enabled_capabilities: []`，即界面上的「全部」选项）正好落在这一态，
  所以每台按默认配的服务器都会中招。
- **复现**：
  1. 存一台 stdio 服务器，`enabled_capabilities: []`（全开）。
  2. `GET /api/extensions/mcp` —— `connected: true`、`protocol_version: 2025-06-18`、
     `server_info` 齐全。
  3. 同一行的 `tool_count` 是 **0**，而 `tools/list` 真的返回了 3 个。
- **修复**：`tools_capability_on` 显式分叉空与非空：
  `None => false` / `Some(v) if v.is_empty() => true` / 其余按名字匹配（大小写不敏感）。
  注释里写明「空数组在这里是『没有例外』，不是『一个都没给』」。
  **为什么不用 `v.is_empty() || v.any(...)` 一句写完**：三态是这个模块的核心口径，
  合并成一行之后下一次有人「顺手简化」就会把这条守卫弄丢，而这正是本次出错的形状。
- **回归**：
  - `mcp_client::tests::the_capability_gate_keeps_all_three_states_apart`（三态逐态断言）
  - `mcp_stdio_protocol::a_real_stdio_server_..._reports_its_tools`（真握手后 `tool_count == 3`）
  - `mcp_stdio_protocol::the_capability_gate_zeroes_the_tool_count_without_faking_a_connection`
    （`connected` 仍为 true 而 `tool_count` 为 0 —— 两件事必须分开报）

---

## ISSUE-012 · 协议层接好之后，前端把「已经接好」说成「还没接」

**状态**：已修（2026-10-06，b 线铺 MCP 协议层时发现）

- **现象**：`devices/api.ts` 里 `connected` 的注释写着
  「协议层（rmcp）还没接，所以现在恒为 `false`。**不要**拿它当装饰」；
  `capabilityGaps.ts` 里 MCP 那一条的 `detail` 写着
  「配置能存能读；协议层（rmcp）未接，tools/list 还拿不到」。
  协议层铺好之后，这两处都成了**假话** —— 界面会告诉用户「一条工具都拿不到」，
  而实际上 `tools/list` 真的拿到了。
- **严重度**：严重（与 ISSUE-009 同一类：界面替后端说谎）
- **根因**：`capabilityGaps.ts` 自己写着「改后端路由或挂接逻辑时必须同步改这里」，
  但没有任何机制强制执行 —— 这次的旧文案是被**前端测试**挡下来的
  （`capabilityGaps.test.ts` 断言 detail 必须提到「协议层」，
  `LibraryTab.test.tsx` 断言渲染出的文案），
  它们本来是防「谎报已接通」的护栏，协议层一落地反而变成了「谎报没接通」的固化器。
- **复现**：
  1. 铺好 rmcp 协议层，`GET /api/extensions/mcp` 返回 `connected: true`、`tool_count: 3`。
  2. 打开「专家库 → 人格原文提到的能力」，MCP 那一条仍写
     「配置能存能读；协议层（rmcp）未接，tools/list 还拿不到」。
- **修复**：
  1. `capabilityGaps.ts` 的 MCP `detail` 改成实话：
     「stdio 服务器真的握手并 tools/list 了；这些工具还没挂进对话的工具表」。
     缺的**换成了下一环**，不是协议层。
  2. `devices/api.ts` 的 `connected` 注释改成现在的语义（真值，且一台都没探测时为 false），
     并新增 `status` / `probed` / `connected_count` / `failed_count` 四个字段。
  3. 界面按 `probed` 把「没查过」与「查了没通」分开显示 —— 这两种的下一步完全不同。
  4. **反向断言也一起改**：`capabilityGaps.test.ts` 那条原本钉「必须说缺协议层」的断言
     改成「必须说缺挂进工具表这一步」，并**新增**一条断言
     `not.toMatch(/协议层（未接|没接|还没）/)` —— 护栏方向要能双向用，
     否则下一次落地新环节时又会被旧断言挡成假话。
- **回归**：`ui/web` 全量 62 passed / 0 failed（其中 `capabilityGaps.test.ts` 6 条、
  `LibraryTab.test.tsx` 6 条）；`i18n-check` 0 问题；`typecheck` 干净。

---

## ISSUE-013 · 门禁的 `failed` 从写出来那天起就恒等于 0

**状态**：已修已回归（2026-10-06）

- **现象**：`.wsl-verify-persona.sh` 打印 `TOTAL passed=N failed=0`，
  而同一次运行里 `cargo` 明确报了 `error: test failed, to rerun pass -p quill-server --lib`。
  实测一次：脚本说 `passed=939 failed=0`，实际有 1 个用例红了。
- **严重度**：阻断（对**纪律**而言）。cron 的硬要求是「全量门禁 0 failed 才算这一步做完」，
  而这个检查**物理上不可能报出任何非零失败数** —— 它一直在给每一次红灯开绿灯。
- **根因**（三个，逐层）：
  1. `awk -F'[ ;]' '{p+=$4; f+=$6}'` 数失败。`-F'[ ;]'` 把 `; ` 当成**两个**分隔符，
     中间多出一个空字段，所以 `test result: FAILED. 151 passed; 7 failed;` 的字段是
     `$4=151 $5=passed $6= $7=7` —— 失败数在 **`$7`**，脚本读的是 `$6`（恒为空），
     `f += ""` 恒为 0。**这不是「某些情况漏报」，是恒等为零。**
  2. 整套测试**跑两遍**（一遍求和、一遍抓失败）。两遍之间代码可能已经变了，
     于是脚本能自己跟自己打架：头一遍 `failed=0`，第二遍 `error: test failed`。
  3. 中途 bail 时（某个测试目标红了，cargo 停住不跑其余目标），
     它照样把「只跑到一半」的条数印成一个笃定的 `TOTAL`。
- **复现**：`echo 'test result: FAILED. 151 passed; 7 failed; ...' | awk -F'[ ;]' '{p+=$4; f+=$6}'`
  → `passed=151 failed=0`。
- **修复**：
  1. 解析改成**按标签找它前面那个数**，不认字段位置（空字段从此不再是问题）。
  2. 只跑一遍，输出、退出码、数字全部来自**同一次运行**；加 `--no-fail-fast`，
     一个目标红了也把其余跑完，否则总数永远是「跑到一半的数」。
  3. 以 cargo 的**退出码**为准；退出码非 0 却一个失败都没数到时，
     判「运行不完整」并退出 1，**不许**把那个不完整的数字当通过率印出去。
- **回归**：`.scripts/gate-selftest.sh`（入库），四个场景（全过 / 真红 / 中途 bail 0 失败 / 连
  `test result` 都没打出来）结论全对，且场景 2 额外钉住**数字本身**
  （`failed=1` 必须被数出来，不能只是碰巧判了失败）。
  **反向验证**：把判定换回旧写法后自测变红（已复现并撤回）——
  旧写法下场景 2 报 `passed=302 failed=0`。
- **附带发现**：门禁脚本 `.wsl-verify-persona.sh` **根本不在版本库里** ——
  `.gitignore` 第 69 行把 `/.wsl-*.sh` 归为「本地临时验证脚本」。
  也就是说，把关全项目「0 failed」纪律的那道检查，**从未被第二个人看过一眼**，
  上面那个恒等于 0 的解析就是这么活下来的。门禁按约定留在本地没问题，
  但「它必须怎么判」得入库，所以自测放进了 `.scripts/`。
- **注意**：本条修的是**门禁自己**，不是产品代码。修好之后 b 线那次真实门禁
  立刻报出 `passed=939 failed=1`（隔壁会话正在改 `mcp_client.rs` 造成的瞬时红）。

---

## ISSUE-014 · 服务器自报「我没有 tools 能力」，界面仍算它有 3 个工具

**状态**：已修（2026-10-06，接 `with_mcp_tools` 时一并修掉）

- **现象**（两个方向，都会说谎）：
  1. 一台服务器在 `initialize` 里自报 `capabilities: {}`（**没有 tools**），
     quill 照样把它的 `tools/list` 结果算成 `tool_count: 3`。
     界面上会显示「3 个工具可用」，而服务器自己说它一个工具都没提供。
  2. 反过来更常见：真实的 MCP 服务器**只提供 resources / prompts**，
     它对 `tools/list` 会回一个 JSON-RPC 错误（`-32601`）。
     现在的 `probe` 会把这条错误当成连接失败，界面上显示成**红色失败** ——
     而这台服务器完全正常，它只是没有工具。
- **严重度**：严重（对用户说谎，且会把人引到错误的排查方向上）
- **根因**：`mcp_client::probe` 只读**本地配置**的 `row.enabled_capabilities`
  （`tools_capability_on`），**从不读 `initialize` 结果里服务器自报的能力**。
  本地开关与服务器自报是两件事，现在只判了前者。
  注意这与 ISSUE-011 **不是同一条**：011 是本地三态被压扁（`Some([])` 全开被判成全禁），
  这一条是服务器自报的那一半根本没人看。
- **复现**（已实测，不是推演）：
  ```
  printf '%s\n%s\n' \
    '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{...}}' \
    '{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}' \
    | ./target/debug/quill-mcp-stub --caps-off
  ```
  实测输出：initialize 回 `"capabilities":{}`（没有 tools），
  紧接着 `tools/list` 照样回了 3 个工具。`probe` 只会把这 3 个算进去。
- **为什么一直没被抓到**：`mcp_stub.rs` 的用法注释里**已经写了 `--caps-off`**，
  但整个测试套件里**没有任何一条测试用到它** —— 一个没人用的测试夹具，
  正好盖住了它本该盖住的那个洞。
- **修复方向**（已定）：`probe` 要把 `ServerPeerInfo.capabilities` 一起纳入判定，
  并把「服务器没这个能力」与「服务器调用 tools/list 失败」**分成两种不同的报告** ——
  前者是正常状态（`connected: true`、工具数 0、附一句白话），
  后者才是失败。修的时候必须给 `--caps-off` 补一条端到端测试。
- **已修（2026-10-06，接 `with_mcp_tools` 时一并做的）**：
  - `Probe` 加 `server_declares_tools: Option<bool>`，从 `initialize` 结果里读
    **服务器自报**的能力（`ServerCapabilities.tools`）。`Probe::not_probed` 记 `None`
    （没报就是没报，不替它猜）。
  - `mcp_client::discover` 改成**两道闸门都要过**才把工具交出去：本地
    `enabled_capabilities`（`tools_capability_on`）**和**服务器自报。
    任一道关掉都报 `tool_count: 0` 且 `tools` 为空。
  - `connected` 仍然是 `true`（这是**正常状态**，不是连接失败），原因写在
    `error` 里并带「下一步」。
  - `call_tool` 在**同一次握手**里也判这道闸门：自报没有 tools 的服务器，
    即便侥幸挂上了工具，直接调也拿不到一句白话而只拿到协议错。
  - `Summary::note` 单独加一句说明「有 N 台自报没有 tools 能力」。
  - 界面上「连上了但 0 个工具」时把 `error` 显示出来 —— 之前那两个分支
    （未探测 / 没连上）都走不到，界面上只剩一句「已连上，0 个工具」，
    用户不知道该改配置还是该换服务器。
- **回归**（真的跑过，`.wsl-verify-persona.sh` 全量 957 passed / 0 failed）：
  - `mcp_stdio_protocol::a_server_that_self_reports_no_tools_capability_reports_zero_not_its_list`
    —— 对着真子进程：`--caps-off` 下 `probed=true`、`connected=true`、
    `server_declares_tools=Some(false)`、`tool_count=0`、`tools` 为空、原因带「下一步」。
    这条同时把**一直没人用的 `--caps-off` 夹具**用起来了（见上面「为什么一直没被抓到」）。
  - `mcp_stdio_protocol::a_server_that_declares_tools_is_mounted_normally`
    —— 与上一条成对：自报有 tools 时 `tool_count=3`、`tools.len()=3`、`error` 为 `None`。
    只测「按 0」的话，一个「永远返回 0」的实现也能过。
  - `mcp_stdio_protocol::calling_a_tool_on_a_server_that_declares_none_is_refused_with_a_next_step`
    —— 调用侧同一道闸门。
  - `ui/web/src/devices/DevicesMcpMount.test.tsx` 的
    「连上了但一个工具都没有时，原因必须显示出来」。

---

## 待补（还没跑到，先占位）

跑 100 条任务时新发现的问题往这里追加，编号接着往下排。
**不要**把新问题混进上面已修的条目里 —— 它们的回归测试不同。
## ISSUE-015 · `tools/call` 的失败原因不带「下一步」，用户在界面上无从动手

**状态**：已修（2026-10-06，写 `with_mcp_tools` 时被新写的测试当场抓住）

- **严重度**：严重（违反红线「面向用户的文案全简体中文；错误必须带『下一步：…』」）
- **现象**（实测，不是推演）：写完 `tools/call` 之后加了一条
  「服务器在 `tools/call` 时退出不许报成功」的测试，断言里顺手要求错误带「下一步」，
  当场红了：
  ```
  ---- a_server_that_dies_on_tools_call_is_not_reported_as_success stdout ----
  面向用户的错误必须带「下一步」：tools/call 失败：Transport closed

  服务器 stderr 末尾：
  stub: 握手来自 "quill"
  stub: 按要求在 tools/call 时退出
  ```
  也就是说：`Transport closed` 这句话会被**原样回灌给模型**（工具结果的 `Err`
  文本进上下文），也会经 `note`/`error` 进界面。模型看到它无从判断该重试还是放弃，
  用户看到它更无从判断该查什么。
- **根因**：`invoke` 里的错误是照着 `handshake` 的既有写法写的
  （`format!("tools/call 失败：{e}")`），而那套写法本来就没带「下一步」——
  这条红线在协议层那一轮（`probe`）就没有被完整执行。加断言才把它暴露出来。
- **已修**：
  - `invoke` 的 `tools/call` 失败分支补上「下一步」，并说明
    **不替它猜原因**（`Transport closed` 不区分「服务器崩了 / 管道断了 /
    它自己关了 stdio」，三者回的都是这一句），只把「怎么查」说清楚 + 附上
    服务器自己的 stderr。
  - `invoke` 的 `initialize` 失败分支同样补上「下一步」。
  - `call_tool` 的「这台服务器这一轮没发起过连接」分支补上「下一步」
    （原来直接拼 `not_probed_reason` 的原文，而「已停用」那句本身没有「下一步」）。
- **回归**：
  - `mcp_stdio_protocol::a_server_that_dies_on_tools_call_is_not_reported_as_success`
    —— 断言 `err.contains("下一步")`。这条断言是**修完之后**才绿的；
    修之前它是红的，红的原因就是本条 ISSUE。
  - `mcp_stdio_protocol::a_server_that_self_reports_no_tools_capability_reports_zero_not_its_list`
    与 `..._declares_none_is_refused_with_a_next_step` 也都断言了「下一步」。

---

## ISSUE-016 · `/healthz` 报 `llm.configured: true`，而模型端点其实连不上

**状态**：待修（2026-10-06 第一次真机起服务时实测发现）

- **严重度**：中等（不是说谎，但很容易被读成「模型能用」）
- **现象**（实测，不是推演）：quill-server 跑在 WSL 里，4B 模型跑在 Windows 上，
  而 `llm::DEFAULT_BASE_URL` 是 `http://127.0.0.1:18080/v1`。WSL 的 `127.0.0.1`
  不是 Windows 的 `127.0.0.1`，于是：
  ```
  GET /healthz  →  "llm": { "base_url": "http://127.0.0.1:18080/v1",
                            "configured": true, ... }      "status": "ok"
  POST …/messages →  503 provider_unavailable
                    「连不上 LLM 服务 …/chat/completions：连接被拒绝，
                      目标端口上没有进程在听」
  ```
  也就是说 `configured: true` 只表示**配置项填了**，不表示**端点活着**。
  照字面读会以为模型是通的，真正报错要等到发第一条消息。
- **根因**：`LlmConfig` 只校验「有没有填」，没有任何一步去探活。
  这个字段名本身没说清自己只是「已配置」。
- **注意（这一条是好的，别改坏）**：那条 503 的错误信封是**合格**的 ——
  如实说了「连接被拒绝」、给了最可能的原因（llama-server 没起）、
  给了可复制的 `curl`，并且 `next_step` 单独成段。红线「错误必须带下一步」
  在这条路径上是满足的，**不要**为了修这条 ISSUE 去动它的措辞。
- **修复方向（未定）**：`/healthz` 里加一个**实测**的可达性字段
  （比如 `llm.reachable` 与 `llm.probe_error`），或者把 `configured`
  改名成不让人误读的名字。**不要**把探活做成阻塞式的 ——
  `/healthz` 会被前端轮询，一次几十秒的探活会把页面拖死。
- **回归**：无（尚未修）。修的时候必须有一条「端点不可达时 `healthz` 如实说不可达」
  的测试，且要真的去连一个不存在的端口，不能拿 `configured` 冒充探活结果。

## ISSUE-017 · README 的「怎么跑」指向 `runner.py`，而那个文件不存在

**状态**：已修（2026-10-06 写出了 `runner.py`，见文末回归）

- **严重度**：低（文档问题，但挡着唯一的「主要路子」）
- **现象**：`TESTSETS/README.md` 的「怎么跑」写的是
  「**浏览器模拟真实用户**（主要路子）：`runner.py` 驱动 MCP 工具面板逐条发任务」，
  而 `TESTSETS/` 下只有 `build_tasks.py`，**没有 `runner.py`**
  （`Test-Path TESTSETS/runner.py` → False）。
- **为什么值得记**：那 100 条的**主要**跑法就是它。它不存在，意味着
  「按 README 跑」这条路从一开始就是断的 —— 而 README 读起来像是跑得通的。
  这与 ISSUE-003/005/012 是同一类病：**界面/文档替后端说了一件没发生的事**。
- **修复方向（未定）**：要么把 `runner.py` 写出来，要么把 README 改成
  说清「目前只能手工逐条跑」。**不许**留着一个指向不存在文件的「主要路子」——
  那比明说「还没做」更坏，因为它让人以为只是没找到。
- **回归**：无（文档没有自动测试）。加一条检查：README 里提到的每个
  `路径/文件` 都必须真实存在。
- **已修**（2026-10-06）：写出了 `TESTSETS/runner.py`。选「把文件写出来」而不是
  改文档，因为除了文件名之外还欠着一件更实的事：**没有能一次跑一批任务的驱动**，
  手点 100 条本身就不现实。README 的描述（驱动 MCP 面板逐条发任务）现在是**真的**。
  纯标准库，无第三方依赖；参数 `--base/--token/--out/--limit/--only/--source/--dry-run`。
- **跑了什么**：
  - `--dry-run` 全量：120 条任务的 skill slug 与 `TESTSETS/skills/*.md`
    **完全对齐**（0 缺 0 多），并逐条报出缺哪几台 MCP 服务器。
  - SkillsBench 真跑 3 条（`sb-3d-scan-calc` / `sb-ada-bathroom-plan-repair` /
    `sb-adaptive-cruise-control`），结果写 `/tmp/quill-sb3b.jsonl`。
  - 顺带把 100 条跑不完这件事**量了出来**，见 `STATUS.md`：
    MCP-Atlas 50 条要 22 台外部服务器，SkillsBench 50 条的**输入夹具不在仓库**
    （`/root/input/*`、`scan_data.stl` 等全缺）。进度仍是 **0/100**。
- **跑的过程中 runner 自己出了个 bug，已修**：原 `verdict()` 把
  「三个维度全部 unjudgeable」判成 **PASS** —— 也就是「什么都没验成」被算成通过。
  这正是红线里最不许发生的那种假通过。现改为独立判成 **`UNJUDGEABLE`**，
  并且「真的跑完」只认 `PASS + PARTIAL + FAIL`。
  `sb-3d-scan-calc` 修前判 PASS，修后判 `UNJUDGEABLE` —— **同一个结果，
  两次判定不同，差的正是这个洞**。
- **回归**：暂无单测（本仓库的 Python 不进门禁）。人工核对项：
  `sb-3d-scan-calc` 现在判 `UNJUDGEABLE` 而不是 `PASS`。

---

## ISSUE-018 · 模型已经回过话了，「下一步」却还在教用户「去确认端点活着 / 启动 llama-server」

**状态**：已修（2026-10-06，真机跑 SkillsBench 时暴露）

- **严重度**：中等（错误本身**如实**转述了上游返回；错的是它附带的处置建议，
  会把用户引到一台根本没坏的机器前面反复排查）
- **红线相关**：面向用户文案必须带「下一步：…」。本条不违反「有没有下一步」，
  违反的是「这个下一步**对不对**」—— 给一句错的下一步，比不给更费时间。
- **现象**（实测，不是推演）：`sb-adaptive-cruise-control` 与
  `sb-ada-bathroom-plan-repair` 两条真机任务，上游原样回了：

  ```
  HTTP 400 {"error":{"code":400,
    "message":"request (8525 tokens) exceeds the available context size (8192 tokens),
               try increasing it",
    "type":"exceed_context_size_error",
    "n_prompt_tokens":8525,"n_ctx":8192}}
  ```

  quill 把这条**如实**转述成了 `provider_unavailable` / HTTP 503，然后说：

  > → 下一步：模型服务不可用：先执行 `curl $QUILL_LLM_BASE_URL/models` 确认端点活着；
  > 本地模型请先启动 llama-server，再用 `QUILL_LLM_BASE_URL` / `QUILL_LLM_MODEL`
  > 指向正确的地址与模型名后重启 quill-server。

  **端点明明是活的** —— 它刚结构化地回了 400，里面连 `n_prompt_tokens`
  和 `n_ctx` 都给了。这句建议让用户去检查一个根本没坏的连接。
- **根因**：`api_chat` 里有**两个** `map_err` 把 `ProviderError` 压成了同一个
  `ApiError::ServiceUnavailable`。而 `ProviderUnavailable::next_step()` 是一条
  固定串（「确认端点活着 / 启动 llama-server」），它只对
  `NotConfigured` / `Unreachable` / `Timeout` 这几种成立。
  `Status(400)` 属于**另一回事**：连上了、回了话、只是被拒。
  两个完全不同的处境共用一句话，是本条的病根。
  detail 里那句 `执行 quill doctor；重试没有意义，请先按上面这句把配置改对`
  同样来自这条固定串，一并错。
- **已修**：
  - `ApiError` 新增变体 `ProviderRejected { detail, advice }`，
    `code()` = `provider_rejected`，`status()` 仍是 **503**（对外形态不变，
    免得已有前端分支失效），但 `next_step()` 返回**调用方按错误种类挑好的**那句。
  - `api_chat` 新增 `provider_failure()` 做分流：连不上 → 走原来的
    `ServiceUnavailable`；**连上了但被拒** → 走 `ProviderRejected`。
    另加 `ADVICE_REJECTED_BY_STATUS`，措辞用「最常见的一种是上下文超了」
    而非断言式说法 —— 因为**没有解析上游 body**，不该替上游断言原因。
  - detail / status 形态保持不变，只改「下一步挂哪一句」，把改动面收在最小。
- **回归**：
  - `error::tests::provider_rejected_keeps_the_same_status_but_a_different_next_step`
    —— 断言连不上与被拒**状态码相同、错误码不同、下一步不同**，并且
    「真连不上」的那句里**必须仍然**含「确认端点活着」
    （别把一个 bug 修成另一个 bug）；反向断言被拒时**不得**再出现
    `llama-server` 三个字。
  - `error::tests::both_provider_outcomes_hand_the_user_a_distinct_actionable_next_step`
    —— 断言两条 `next_step` 互不相同、长度 > 20、含可执行动作。
    **这里刻意不断言字面量「下一步」三个字**：`next_step` 是结构化字段，
    前端把它渲染成独立段落（`Page.tsx` 的 `.form-error-next`），标签由字段名承担，
    且仓库既有的 `unauthorized` / `too_many_requests` / `internal` 三条
    `next_step` 都不含这三个字 —— 要断言它就得先改那三条，属于无谓的措辞 churn。
  - 全量 `.wsl-verify-persona.sh`：**959 passed / 0 failed**。
- **真机复跑**（重新 `cargo build` → 重启 quill-server → 重跑
  `sb-adaptive-cruise-control`，结果在 `/tmp/quill-fix-018.jsonl`）：
  ```
  修前  code=provider_unavailable
        → 下一步：…确认端点活着…启动 llama-server…          ← 端点明明是活的
  修后  code=provider_rejected
        → 下一步：模型服务**活着**并回了一个错误状态码，它自己的原话在上一段里。
          下一步：照那句话改，**不要**去重启模型服务（它正在正常应答）。
          最常见的一种是请求超出了模型上下文：调大 QUILL_LLM_MAX_CONTEXT_TOKENS，
          或减少这一轮挂着的技能/工具…改完用同一条消息重试；细节跑 `quill doctor`。
  ```
  错误码与建议都在真服务上验过，不是只有单测绿。
- **残留（不算本条修完）**：detail 里**还嵌着另一句**旧的「下一步：执行
  `quill doctor`；重试没有意义，请先按上面这句把配置改对」，界面上会渲染成
  第二个段落。用户看到两条下一步，一条含糊一条具体，且含糊那条的
  「上面这句」现在指的是上游那段 JSON。**另立 ISSUE-020 跟。**

---

## ISSUE-019 · 挂几个技能就把请求撑爆 8192 上下文，而 quill 不预检、也不点名真因

**状态**：待修（2026-10-06，与 ISSUE-018 同一次真机跑暴露；本轮只记录，未修）

- **严重度**：中等（用户会看到「这条任务发不出去」，但拿不到「为什么」和「怎么办」）
- **现象**（实测数字，全部来自 `TESTSETS/tasks.json` 与磁盘上的真实技能正文）：

  | 任务 | prompt 字符 | 挂的技能 | 技能正文字符合计 | 合计 | 上游实报 |
  |---|---|---|---|---|---|
  | `sb-adaptive-cruise-control` | 2579 | 5 个 | 9375 | 11954 | `n_prompt_tokens=8525` |
  | `sb-ada-bathroom-plan-repair` | 9459 | 3 个 | 13935 | 23394 | `n_prompt_tokens=9804` |

  两条的 `n_ctx` 都是 **8192**。也就是说：**装上技能**之后，请求本身就已经越界，
  根本轮不到模型作答 —— 用户看到的是一条与「我这条任务问的是什么」毫无关系的失败。
- **别把它算成一件事的两倍**（这条容易讲歪，如实说）：
  - `sb-adaptive-cruise-control` 里，**技能占了输入的 78%**（9375 / 11954）。
    不挂技能只发 prompt 约 2579 字符，是装得下的 —— 这里**确实是挂技能撑爆的**。
  - `sb-ada-bathroom-plan-repair` 的 prompt 自己就有 9459 字符，**它自己就已经越界了**。
    这里挂技能只是让情况更糟，**单归到「技能撑爆」是不诚实的**。
- **根因**：`ISSUE-007` 说「SKILL 正文整段进工具描述，每轮请求都要为它付 token」。
  本条是它的**后果面**：付到什么程度没人量过、也没有任何地方拦一下 ——
  quill 不估 token、不看上下文窗口、直接把可能越界的请求发出去，
  然后由上游用一句 `exceed_context_size_error` 兜底，再由 ISSUE-018 那条
  错误的建议把用户支到「去确认端点活着」。三个问题串成一条失败路径。
- **查过了，别冤枉它**：quill 确实有 `compaction_threshold_tokens` 字段
  （实测 `/healthz` 报 `8000`，而这条失败请求是 8525 token ——
  看着像「阈值设了却没拦住」）。**但那不是 bug，是诚实披露**：
  `ChatPage.tsx` 的界面原文就写着「已配置压缩阈值 {{threshold}} tokens，
  **但压缩还没实现**：超过上限不会自动摘要，需要自己新建会话」。
  仓库里也确实没有任何压缩实现（`quill-agent` 搜 `compact` 零命中）。
  所以这条 ISSUE 的准确说法不是「阈值失灵」，而是：
  **本地没有任何拦截点**，唯一的兜底在上游，而上游兜底完还会被 ISSUE-018
  带偏一次。
- **修复方向（未定）**：发请求前做一次 token 预检（超了就**在本地**拦下来，
  并点名「是这些技能的正文把请求撑大了：<slug 列表>」），而不是等上游回 400。
  真正的根治是 ISSUE-007 本身（别把整段正文塞进工具描述）。
  **本轮不修**：修它要动工具描述的形态，超出本轮范围。
- **回归**：修的时候必须有一条「预检挡住越界请求时，报错里要列出是哪些技能撑大的」。
  4B 模型答不上来不算 bug，但**请求压根发不出去**算。

---

## ISSUE-020 · 一个错误信封里有**两条**「下一步」，其中一条还藏在 detail 里

**状态**：待修（2026-10-06，验证 ISSUE-018 的修复时撞见）

- **严重度**：低到中等（信息都在，但互相打架；用户不知道该照哪句做）
- **现象**（真机实测，`sb-adaptive-cruise-control` 修复后的返回）：
  `Page.tsx:65-70` 把 `detail` 与 `next_step` 渲染成**两个独立段落**，
  而 `detail` 本身是由 `format!("模型调用失败：{e}")` 拼的，
  `{e}` 是 `ProviderError` 的 `Display`，而它的 `tail()` 会追加
  「→ 下一步：执行 `quill doctor`；…」。于是界面长这样：

  ```
  模型调用失败：LLM 服务返回 HTTP 400：{"error":{…"n_prompt_tokens":8525…}}
  → 下一步：执行 `quill doctor`；重试没有意义，请先按上面这句把配置改对

  模型服务**活着**并回了一个错误状态码，它自己的原话在上一段里。下一步：照那句话改，
  **不要**去重启模型服务…最常见的一种是请求超出了模型上下文…
  ```

  两句都自称「下一步」。第二句（结构化的那个）是对的；第一句含糊，而且它说的
  「上面这句」现在指的是**上游那段 JSON**，不是某个配置项。
- **根因**：`next_step` 既是**结构化字段**，又已经被拼进了 detail 的文本里 ——
  同一件事有两个出口，而其中一个出口没跟着 ISSUE-018 一起改。
  这不是 ISSUE-018 引入的（改之前就是两条），但 ISSUE-018 修完之后它更显眼了：
  现在两句的**具体程度差得很远**，用户更容易只盯着结构化那句而忽略 detail 里的。
- **修复方向（未定）**：让「下一步」只有**一个**出口。最直接的做法是
  `ProviderError` 的 `Display` **不再**拼 `→ 下一步：…`，
  下一步统一由 `ApiError::next_step()` 提供 —— 那个字段本来就是为此存在的。
  改之前要盘一遍有多少调用点把 `ProviderError` 直接 `to_string()` 之后当用户可见文案用，
  那些地方需要显式补 `next_step`，**不许**顺手把下一步删干净。
- **回归**：一条断言「`detail` 里不含 `→ 下一步`」的测试，外加一条
  「`ProviderError` 直接 `to_string()` 之后仍带得上下一步」的测试 ——
  钉住的是「只有一个出口」，不是「没有下一步」。

---

