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

**状态**：代码已修 + 单测已过 + 负向验证已过；**浏览器真机复验待做**（批量重跑正占着服务端）

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
- **根因**：`servers = draft?.servers ?? mcpServersOf(config.data)` ——
  草稿是前端本地状态，它**整份替换**了服务端返回的列表再渲染，
  渲染层拿不到「这一行服务端到底有没有」。顶部那句「已保存配置，尚未连接验证」
  说的是**连接状态**，没有一句在说这行本身还没存。
- **已修**（`ui/web/src/devices/`）：
  - `api.ts` 新增纯函数 `mcpRowState(row, saved, hasDraft) -> 'saved' | 'changed' | 'new'`：
    只拿**名字 + 内容**与服务端那份比。`new` = 服务端根本没有；`changed` = 有同名但内容不同；
    `saved` = 逐字段一致。
    - `hasDraft === false` 时**恒为 `saved`** —— 把「服务端归一化过的字段与表单字段长得不一样」
      这类误判，限制在「用户手上正有未保存改动」这个窗口内。
    - `undefined` / `null` / `''` 在可选字段上归为同一件事；`env` / `headers` 按键排序比较，
      字段顺序不该被当成「用户改过了」。
  - `Devices.tsx` 两个页面（`DeviceListPage` 与 `DeviceMcpPage`）都接上：
    1. 草稿行**在自己身上**挂「未保存（新增）」/「未保存（有改动）」标记；
    2. 顶部横幅写清「有 N 条改动还没保存，关掉这个标签页就丢了。下一步：点右上角「保存 MCP 配置」」；
    3. **按钮文案跟着状态变**：`new` → 「丢弃草稿 X」（那个按钮只动本地草稿，
       说「删除」就是骗人）；`changed` → 「删除 X（保存后才生效）」；`saved` → 原「删除 X」。
  - 样式加在 `layout_local.css`，**没有动 `index.css`**（vendor 移植基准，SHA256 保持不变），
    只用 vendor 已有的 CSS 变量，不新增色值。前端依赖仍是 8 个。
- **回归**：
  - 新增 `src/devices/DevicesDraftRows.test.tsx`，**10 条**：
    - 无草稿时渲染卡片数 **==** 服务端条数、0 个标记、每行都明确落在 `saved` 上；
    - 加入草稿后：旧的两行仍是 `saved`，只有新行是 `new` 且带标记（**标记长在那一行自己身上**）；
    - 草稿行按钮说「丢弃草稿 X」，已保存行仍是「删除 X」；
    - 横幅含「1 条改动还没保存」与「下一步」；
    - 改已存在的一行 → `changed` + 「有改动」+「保存后才生效」，**且没动过的那行不许跟着变**；
    - `mcpRowState` 三态 + `hasDraft=false` 恒 saved + undefined/null/'' 归一 + env 键顺序无关。
  - 全量：typecheck 0 错、**vitest 85 passed**（原 75，+10）、i18n 问题数 0、
    library 334/334、依赖 8、`index.css` SHA256 未变。
- **负向验证**：把 `mcpRowState` 改成恒 `return 'saved'`（那正是 ISSUE-010 本身的行为），
  如期有 **6 条变红**；破坏与回滚都在脚本内，`api.ts` 已确认复原。
- **还没做到的验证，如实说**：
  - **浏览器真机复验没做成**。本轮 50 条 SkillsBench 重跑正占着服务端，
    react-query 反复重取导致 DOM 节点不停被替换：`fill` 先是 275s 超时，
    之后连着三次 `STALE_ELEMENT_REF`。这种状态下点出来的结果不可信，
    **不能**拿它当「已验证」。已构建好 `dist`，等批量跑完后再真机复验：
    加入草稿 → 新卡片带「未保存（新增）」且按钮为「丢弃草稿」→ `curl` 确认服务端仍无此行。

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

## ISSUE-030 · `message_count` 与真实消息条数对不上 —— 88 个会话里 64 个是错的

**状态**：待修（2026-10-06 用户登录后在对话页实测发现）

- **现象**：用户没选角色直接发消息，对话区只看到自己发出去的那条，没有回复，
  会话列表与标题栏显示「N 条消息」。**这个 N 是错的。**
- **严重度**：严重（界面上一个对不上的数字，且用户会拿它判断「到底发出去没有」）
- **实测数据**（`/tmp/quill-browser-round/quill.db`，真库不是推断）：
  ```
  有消息的会话数: 88    message_count 与真实条数对不上的: 64
  漂移形态：记 0 实际 1（绝大多数）
  ```
  本次那个会话：库里 3 条（user×2 + assistant×1），
  `sessions.message_count` 记 **2**，界面就照着显示「2 条消息」。
- **根因**：`api_chat.rs:929` 的 `touch_session` 是
  `message_count = message_count + 2`（一条用户消息 + 一条助手回复），
  **只在请求成功走到助手回复那一步时才执行**。一轮失败时：
  用户消息**已经写进库了**（`INSERT INTO messages` 在前），
  但助手回复没生成、UPDATE 也没跑 —— 于是消息留了、计数没加。
  这正是「只有发出去的消息、没有回复」在库里的形状。
- **复现**：
  1. 新建会话，发一条会让这一轮失败的消息（本机 4B 最容易触发的是烧光 4 轮工具预算）。
  2. `select message_count from sessions` → 0。
  3. `select count(*) from messages where session_id=?` → 1。
  4. 界面显示「0 条消息」，而对话区明明有一条自己的消息。
- **修复方向**（未定）：
  1. `message_count` 不该由调用点手工 `+2`，而应在写完消息后按
     `select count(*) from messages where session_id=?` 回填 —— 反规范化字段
     用反规范化方式维护，迟早会漂。
  2. 或者：失败时也把这条用户消息计入，并在会话状态上体现「这一轮没有回复」。
  3. 无论选哪个，都要有一条测试钉住：**`message_count` 必须等于该会话消息表里的真实条数**，
     且要覆盖「一轮失败之后」这个分支 —— 现在一条失败路径的测试都没有。
- **回归**：无（尚未修）。

---

## ISSUE-031 · 发完消息视图不滚到底，回复在屏幕外 —— 用户以为「模型没回」

**状态**：待修（2026-10-06 用户登录后实测）

- **现象**：发一条消息后，界面停在原来那条长消息的顶部不动。
  新发出去的消息和模型的回复都在下面，**要手动滚动才看得到**。
  用户的原话是「基本都只有发出去的消息，本地 LLM 没有回复」。
- **严重度**：严重（把「模型回了」误报成「模型死了」，直接把人引到错误方向）
- **实测**：模型**确实回了** —— 本次用「用一句话回答：1+1 等于几？」验证，
  服务端返回 200、耗时 3107 毫秒、transcript 里是
  `Quill · 07:04 · 1+1 等于 2。（3107 毫秒 · 入 2687 / 出 9 tokens）`。
  三条消息在 DOM 里全都在，**只是没滚进视口**。
- **根因**：`ChatPage` 在追加消息后没有把 `.chat-transcript` 滚到底。
  首屏那条 runner 的 BGP 提示词有 1400 多字，把新消息整个顶出了视口。
- **修复方向**：发送后与收到回复后各滚一次到底；且**滚动必须发生在
  DOM 更新之后**（否则又滚回原位）。
  顺带值得考虑：回复区与输入框之间给一条「新回复在上方」的锚点，
  别让长消息把新内容顶没。
- **回归**：无（尚未修）。要一条测试钉住「发送后 transcript 的 scrollTop
  等于 scrollHeight」。

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
- **残留**：detail 里**还嵌着另一句**旧的「下一步：执行
  `quill doctor`；重试没有意义，请先按上面这句把配置改对」，界面上会渲染成
  第二个段落。用户看到两条下一步，一条含糊一条具体，且含糊那条的
  「上面这句」现在指的是上游那段 JSON。→ 另立 ISSUE-020 跟，
  **ISSUE-020 也已修**（detail 改用 `ProviderError::message()`，不再拼 tail）。

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
- **修复方向（已作废，见 ISSUE-021）**：早先写的是「发请求前做一次 token 预检，
  超了就**在本地**拦下来，并点名『是这些技能的正文把请求撑大了：<slug 列表>』」。
  **这条方向不能用**：`/healthz` 报的 `max_context_tokens=32768` 是配置值，
  而同一个部署的模型真实窗口是 8192（ISSUE-021）—— 估出 8525 < 32768，
  预检会放行，上游照样拒。要走预检这条路，先得解决「窗口从哪来」。
  真正的根治仍然是 ISSUE-007 本身（别把整段正文塞进工具描述）。
  **本轮未修。**
- **回归**：修的时候必须有一条「预检挡住越界请求时，报错里要列出是哪些技能撑大的」。
  4B 模型答不上来不算 bug，但**请求压根发不出去**算。

---

## ISSUE-020 · 一个错误信封里有**两条**「下一步」，其中一条还藏在 detail 里

**状态**：已修（2026-10-06）

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
- **已修**（2026-10-06）：**没有**照上面那个方向直接砍 `Display` 的 `tail()`，
  因为砍了会让流式 / CLI / agent 内部那些**没有 `ApiError::next_step()` 可用**的
  调用点丢掉下一步 —— 那正是红线里最不许发生的那种「顺手删干净」。
  改成加一个只带正文、不带 tail 的出口：
  - `quill-provider`：新增 `ProviderError::message()`，只返回「发生了什么」；
    `Display` 改为 `write!(f, "{}", self.message())? + tail(f, self)`，
    **行为一个字没变**。让 `Display` 复用 `message()` 而不是两边各写一份，
    是为了两个出口以后不会各改各的、跑偏。
  - `quill-server`：`api_chat::provider_failure` 的 detail 改用 `e.message()`。
    全仓核过，`ProviderError → ApiError` **只有**这一处转换（两个调用点都走它）。
- **回归**：
  - `quill_provider::error::tests::message_drops_the_next_step_but_display_keeps_it`
    —— 8 个变体逐个断言：`message()` **不含**「下一步」、
    `to_string()` **仍含**「下一步」、且 `Display` **以 `message()` 开头**
    （钉住的是「只有一个出口」，不是「没有下一步」）。
  - `api_chat::tests::detail_carries_no_next_step_so_the_envelope_shows_only_one`
    —— detail 不含「下一步」，但**必须仍带上游原话**（`n_prompt_tokens`），
    那是用户唯一能照着改的权威依据。
  - `api_chat::tests::the_structured_next_step_is_the_one_that_survives`
    —— 结构化那句自带「下一步」标签，且**不含** `llama-server`（ISSUE-018 那条）。
  - `api_chat::tests::unreachable_still_tells_the_user_how_to_check_the_endpoint`
    —— 连不上时那句「确认端点活着」**必须还在**（别把一个 bug 修成另一个）。
- **这几条不是空断言，已验证**：临时把 `provider_failure` 退回 `{e}` 之后，
  上面 4 条里 **2 条立刻变红**，报错打出来的正是真机上看到的那段重复文本
  （「…n_prompt_tokens:8525}}\n→ 下一步：执行 `quill doctor`…」），
  退回修复后重新变绿。
- **真机复跑**（重新 `cargo build` → 重启 quill-server → 重跑
  `sb-adaptive-cruise-control`，结果在 `/tmp/quill-fix-020.jsonl`）。runner 打印的是
  `detail / next_step`，所以同一处对比最直观：

  ```
  修前  …"n_ctx":8192}}
        → 下一步：执行 `quill doctor`；重试没有意义，请先按上面这句把配置改对
        / 下一步：模型服务**活着**并回了一个错误状态码…              ← 两条

  修后  …"n_ctx":8192}}
        / 下一步：模型服务**活着**并回了一个错误状态码…              ← 只剩一条
  ```

  上游那段 JSON 原样留在 detail 里（`n_prompt_tokens` / `n_ctx` 都在），
  「下一步」只剩结构化那一个出口。
- 全量 `.wsl-verify-persona.sh`：**963 passed / 0 failed**（改前 959，新增 4 条）。

---

## ISSUE-021 · 界面把「配置里写的上限」当成「模型的上限」告诉用户，而它是错的

**状态**：已修文案（2026-10-06）；**探活本身未修**

- **严重度**：中等偏高（直接踩红线「不许伪造任何数据」——
  它没有编造数字，而是把一个**配置值**说成了**实测事实**）
- **现象**（实测，三个数字都是真跑出来的）：
  ```
  GET /healthz  →  "llm": { …, "max_context_tokens": 32768, … }

  同一个部署里的模型端点，真实窗口是：
  HTTP 400 {"error":{…,"message":"request (8525 tokens) exceeds the
            available context size (8192 tokens)…",
            "n_prompt_tokens":8525,"n_ctx":8192}}
                                 ^^^^^^^^  模型说自己是 8192
  ```
  `32768` 是 `admin_config.max_context_tokens` 里**用户填的数**，
  quill 从来没去问过模型。这两个数差了 4 倍。
- **界面怎么说的**（`ChatPage.tsx` 的 token meter，两处）：
  - 行内标签：`本次 {{used}} / 上限 {{limit}}` → 把 32768 印成「上限」。
  - 悬浮提示：`上下文上限 {{limit}} tokens（来自 /healthz）` → 直接断言它是上限。
    这句尤其容易骗到人：它**给了出处**，却把「配置值」讲成了「事实」。
- **为什么会漏掉**：`max_context_tokens` 是个配置项，`llm.rs` 只校验它 `> 0`、
  且不小于压缩阈值与出参预算，**没有任何一步去核对模型的真实窗口**。
  这与 ISSUE-016 同一类病：那次是 `configured: true` 装成「端点活着」，
  这次是「配了 32768」装成「窗口有 32768」。
- **已修**（文案，2026-10-06）：把「上限」这个**断言事实的词**换掉，并说清边界：
  - 行内标签：`本次 {{used}} / 配置上限 {{limit}}`
  - 悬浮提示：`已配置上下文上限 {{limit}} tokens、压缩阈值 {{threshold}} tokens ——
    这两个都是 /healthz 里的配置值，不是模型实测值。模型端点的真实窗口可能更小，
    quill 不去猜它：超了会被模型服务直接拒绝。压缩尚未实现，超过上限不会自动摘要，
    需要自己新建会话。`
  - 中英文同步，`ChatPage.tsx` 的 `defaultValue` 兜底文案一起改
    （否则 i18n 兜底会说出旧的话）。
  - **为什么改文案而不是去探活**：探活要么阻塞（`/healthz` 被前端轮询），
    要么依赖模型端点并不保证提供的字段（`/v1/models` 普遍不报窗口）。
    在那之前，**说「这是配置值」是唯一诚实的选项**。
- **回归**：
  - `node .i18n-check.mjs`：**问题数 0**（`{{used}}/{{limit}}/{{threshold}}`
    与语言包一致）。
  - `tsc -b`：0 错。`vitest run`：68/68。`.library-check.mjs`：334/334。
  - **产物层逐字核对**（`npm run build` 后比对 `dist/assets/index-*.js`）：
    新文案「配置上限」「不是模型实测值」「quill 不去猜它」**都在**；
    旧文案「本次 {{used}} / 上限 {{limit}}」「但压缩还没实现」「来自 /healthz」
    **都不在**。
  - **没做到的一条，说清楚**：token meter 在**登录之后**的聊天页才渲染，
    而登录仍卡在你接管那一步，所以这次**没有在浏览器里肉眼看那个悬浮提示**。
    上面四条是产物层与静态检查的证据，**不是**「界面已肉眼验过」。
- **未修（别当成已修）**：quill **仍然不知道**模型的真实窗口。要真知道，
  得在 `provider.models()` 之后另找一条能报窗口的途径，或让用户在模型服务侧确认。
  → 这条已被 ISSUE-022 部分推翻，见那里。
- **这条把 ISSUE-019 的修复方向改掉了**：原方向是「发请求前拿
  `max_context_tokens` 做 token 预检」。按本条实测，**那条路在当前部署里
  根本不会触发** —— 估出 8525 < 32768，预检放行，上游照样拒。
  所以 ISSUE-019 的预检**不能**被说成修好了它；要修得先解决「窗口从哪来」。→ **这一条已被 ISSUE-022 推翻**：`/v1/models` 一直就报
  `meta.n_ctx`，能力本来就有，是 `CONTEXT_KEYS` 取错了字段。见 ISSUE-022。

---

## ISSUE-022 · 探测到的「上下文窗口」取的是**训练窗口**，比真实窗口大 32 倍

**状态**：已修（2026-10-06）

- **严重度**：高。这是本项目到目前为止**最接近红线**的一条 ——
  它不是编造数字，而是把一个**真实上报的数**放进了**错误的字段**，
  然后界面照着它显示、`validate()` 照着它指导用户填配置。
- **现象**（真机探测原文，2026-10-06，llama-b10068 + Qwen3.5-4B）：
  ```
  GET /v1/models → 200
  "meta":{ … ,"n_ctx":8192,"n_ctx_train":262144, … }
            ^^^^^ 服务窗口     ^^^^^^^^^^^^ 训练窗口
  ```
  llama.cpp 用 `-c 8192` 起的服务器，**同时**报这两个数。
  旧代码 `CONTEXT_KEYS = ["n_ctx_train", "context_length", "max_model_len"]`
  把 `n_ctx_train` 排在**第一位**，`first_positive_u32` 取第一个命中，
  于是探测报出 **262144** —— 比真实窗口**大 32 倍**。
- **为什么会流到用户眼前**（两条路，都是真的）：
  1. `GET /api/admin/providers/{id}/models` 的 `ModelCard.context_window`
     → 模型页照着显示。
  2. `llm_providers.rs:132` 的校验文案：
     「`max_context_tokens` 必须 > 0。下一步：填模型真实的上下文窗口
     （可先用 GET /api/admin/providers/{id}/models 看探测到的 context_window）」
     —— 界面在**教用户照一个偏大 32 倍的数**填配置。
- **这不是手滑，是一个被明确写下来的错误决定**（所以更值得记）：
  旧测试叫 `context_window_prefers_the_trained_length_and_never_invents_one`，
  断言写着「训练上下文优先于实例上下文」和「n_ctx 不是训练上下文，不许拿它顶包」。
  当时的理由是「训练上下文才是模型真正的能力上限，实例开小是部署的事」。
  **那个理由在「模型能力」问题上成立，在「这条请求最多能带多少 token」问题上
  完全不成立** —— 而这个字段正是后一个问题。这是一条被测试**保护**着的错误，
  所以光靠跑测试发现不了；它是真机跑任务、看到 8192 vs 32768 才顺藤摸出来的。
- **已修**：
  - `CONTEXT_KEYS` 改为 `["n_ctx", "context_length", "max_model_len"]`，
    **删掉 `n_ctx_train`**，并把「为什么顺序本身就是判断」「为什么排除它」
    写在常量旁边的注释里。
  - 只报 `n_ctx_train` 的端点现在得到 `None`（**不知道**）而不是那个偏大的数。
    这是刻意的：`None` 让界面说「上游没报这个信息」，而报一个错的数会让用户
    照着把配置改坏 —— 后者远比前者麻烦。
- **回归**：
  - `llm_providers::tests::context_window_is_the_serving_window_never_the_trained_one`
    （由旧的 `context_window_prefers_the_trained_length_and_never_invents_one`
    改名并**翻转断言**）。用真机抓到的原文做夹具，四个条目分别断言：
    `n_ctx + n_ctx_train` 并存 → 取 **8192**；只有 `n_ctx` → 取 8192；
    `max_model_len` → 取 131072；**只有 `n_ctx_train` → `None`**。
    另有一条 `assert_ne!(…, Some(262144))` 单独钉住「不许报 32 倍」。
    测试名与注释都留了「这条断言被推翻过一次」的痕迹。
  - `http_contract::model_probe_derives_name_context_and_modality_from_the_upstream`
    —— HTTP 契约层**同一处错误断言有两处**（探测接口 + 模型池），
    两处都得改，只改一处会让 `cargo_rc=101` 卡住。夹具 `FAKE_LLAMA_MODELS`
    照抄真机原文（`n_ctx: 8192, n_ctx_train: 262144`），所以这条契约测试
    就是把真机行为钉在 HTTP 层。
- **真机验过**（重新 `cargo build` → 重启 quill-server → 打真接口）：
  ```
  GET /api/admin/models   →  ctx_win : 8192      ← 修前这里是 262144
  GET /api/admin/config   →  max_context_tokens = 32768   （对照组：用户填的值，没动）
  ```
  两个数并排看最清楚：**探测到的真实窗口 8192**，**配置里写的 32768**。
- 全量 `.wsl-verify-persona.sh`：**963 passed / 0 failed**。
- **未做**：没有改 `validate()` 里那句「可先用 …/models 看探测到的
  context_window」—— 现在它指的是**正确的**数了，那句话不用动。
- **对 ISSUE-019 的影响**：窗口现在**查得到**了（本部署 8192），
  所以「拿真实窗口做本地预检、超了就在本地拦下来并点名是哪些技能撑大的」
  这条修复方向**重新变得可行**。但仍**未修**，且要注意 `max_context_tokens`
  配错时依然会骗过预检 —— 预检该用**探测到的窗口**，不是配置值。

---

## ISSUE-023 · 同一块界面上两个数都叫「上下文」，一个 32768 一个 8192，谁也不说谁

**状态**：已修（2026-10-06）

- **严重度**：中等（两个数本身都是真的，但没有任何一处告诉用户它们打架了）
- **现象**（ISSUE-022 修好探测之后，界面上真实的样子）：
  - provider 卡片：`上下文: 32768` —— 这是**配置里填的**值，却顶着「上下文」这个
    断言事实的标签。
  - 模型池里的模型标签：`上下文 8192` —— 这是**探测到的**真实窗口。
  - 同一块页面、同一个 provider，**两个不同的数、两个一模一样的标签**。
- **为什么值得单独记**：ISSUE-021 修的是聊天页的 token meter（说「配置上限」），
  那条已经诚实了；但**模型页这一处没跟着改**，于是「同一个产品里，
  一个地方说『这是配置值』、另一个地方说『这是事实』」。修一半比不修更让人困惑。
- **后果**：用户按 32768 去理解自己的配置，于是**请求被 8192 的窗口拒掉**
  （`exceed_context_size_error`）时，完全看不出根因是自己的配置写大了 4 倍。
  ISSUE-019 记的那两条 SkillsBench 失败，正是这么来的。
- **已修**：
  - provider 卡片那个标签：`上下文:` → **`配置上下文:`**（新 i18n key
    `models.configuredContextLabel`，中英同步）。
  - 模型池每个 provider 分组下新增一条**对账提示**：当**探测到的窗口 <
    配置的窗口**时，明写两个数，并给出可执行的下一步
    （把「上下文长度」改成探测值，或调大模型服务的 `-c`）。
- **三条判断刻意这么定，写在 `contextMismatch` 的注释里，也钉进了单测**：
  1. **只在「配置 > 探测」时报**。反过来的（配置比实际小）不会让请求被拒，
     只是白占窗口，报出来是噪音。
  2. **探测值为 `null`（上游没报）时不报**。那是「不知道」，不是「一致」——
     把「不知道」当成没问题，是另一种谎。
  3. **配置值为 `null` 或 0 时不报**。没有可对账的东西。
- **回归**：
  - 新增 `src/models/ModelsPage.test.tsx`，**7 条**，全部围绕上面三条判断，
    夹具里的数字直接用真机实测值（32768 / 8192）。
  - **不是空断言**：把 `probed < configured` 临时改成 `>` 之后，
    7 条里**立刻 3 条变红**（含「报出配的比能吞的大」这条主用例），改回后恢复。
  - `tsc -b`：0 错。`vitest run`：**75 passed**（68 → 75，新增 7）。
  - `node .i18n-check.mjs`：问题数 0（zh key 567→569）。
  - `.library-check.mjs`：334/334。依赖仍 8。`index.css` SHA256 未变。
  - Rust **963 passed / 0 failed**。
- **未做**：这条提示只在**模型页**出现。聊天页的 token meter 仍只读
  `/healthz` 的配置值（ISSUE-021 已把它标成「配置上限」）。
  要让聊天页也知道探测值，得给 `/healthz` 加一个**实测**字段 ——
  但 `/healthz` 被前端轮询，不能在请求里同步探活（见 ISSUE-016 的教训），
  得配一个带 TTL 的缓存，且缓存年龄要在界面上显示。这件事**本轮没做**。
- **没做到的验证**：模型页需要登录才能看到，而登录仍卡在用户接管那一步，
  所以这条提示**没有在浏览器里肉眼看过**。上面是单测 + 产物层
  （`dist` 里「配置上下文」「比你配置的」「models-context-mismatch」都在）
  的证据，**不是**「界面已肉眼验过」。

---

## ISSUE-024 · `tasks.json` 的 id 根本不是唯一键：100 条任务其实只有 66 个可区分的

**状态**：已修（2026-10-06）

- **严重度**：高（**直接影响「100 条 / 100/100」这个完成条件本身可不可信**）
- **现象**：准备跑一批任务时按 id 取条目，发现 `atlas-689bd255c042` 在前 6 条里
  出现了**两次**。全量一数：
  ```
  tasks.json 总条数 : 100
  唯一 id           : 66
  冲突的 id 组      : 10 组（atlas-689bd255c042 出现 9 次，…）
  ```
- **那 9 条不是重复行，是 9 条完全不同的任务**：prompt 不同、`required_tools`
  不同、依赖的服务器也不同（airtable / national-parks / whois / oxylabs …）。
  也就是说 `atlas-689bd255c042` 压根**不是唯一键**。
- **根因**（`build_tasks.py:53`）：
  ```python
  "id": f"atlas-{row['TASK'][:12]}",   # ← 截断到 12 位
  ```
  MCP-Atlas 的 `TASK` 是 **24 位十六进制**，而且是**结构化**的：
  前 12 位是分组前缀，**后 12 位才是序号**。实测：
  ```
  689bd255c0422b257e7dfca5
  689bd255c0422b257e7dfca8   ← 不同的任务
  689bd255c0422b257e7dfca9   ← 不同的任务
  ```
  截到 12 位**恰好只留下分组前缀、把唯一的那半截扔了**。
  原始 500 行里 `TASK` 全部唯一（500/500），截断后只剩 **32 个** id，498 行撞车。
  这**不是**哈希碰撞，是一个会稳定复现的截断错误。
- **为什么这条最该记**：它让三件事同时静默出错 ——
  1. 「100 条任务」其实只有 66 条可区分；按 id 记进度会**虚增**。
  2. 按 id 跑某一条会跑到**另一条**身上。实测：`--only` 给了 13 个 id，
     runner 实际跑了 **34 条**（因为一个 id 匹配到了多条）。
  3. `results.jsonl` 按 id 记录时，不同任务的结果会混在同一个键下。
- **现场复现（不是推演）**：批量跑时，`atlas-689bd255c042` 这个 id
  **连续三次**出现在结果里，第 6/7/8 条的 `required_tools` 分别是
  alchemy+calculator+context7+exa+git / airtable+exa+pubmed /
  alchemy+airtable+memory+slack+whois —— 三条不同的任务，同一个 id。
- **已修**：
  - `build_tasks.py` 改为用**整个** `TASK` 做 id，不截断。
  - `main()` 里加一条**唯一性断言**：id 有冲突就**如实报错、不写 tasks.json**。
    这条断言是被这次事故逼出来的 —— 这类错静默通过，跑得越多越看不出来，
    所以要在生成时就当场拦住。
  - 重新生成后：`总条数 100 / 唯一 id 100 / 无重复`，
    `TESTSETS/skills/` **逐字节未动**（diff 正好只有 50 行 id 变更）。
- **回归**（断言不是摆设，已验证）：
  - 临时把 id 退回 `[:12]` 之后跑 `build_tasks.py`：`rc=1`，输出
    「id 不唯一，共 10 组冲突……如实报错，不写出 tasks.json」，
    且 **`tasks.json` 的 md5 没变** —— 报错时确实没有落盘。
  - 恢复修复后：100 条 / 100 唯一 id。
- **注意**：已经跑过的那批结果**不能作数**（`/tmp/quill-batch1.jsonl`），
  里面的 id 有歧义。要重跑。

---

## ISSUE-025 · runner 不做任务隔离：上一条挂的技能与服务器会漏进下一条

**状态**：已修（2026-10-06）

- **现象**（实测）：
  跑第 7 条（一条 **MCP-Atlas** 任务，`required_tools` 里没有任何技能依赖）时，
  模型报错的原文是：
  > `工具「csv-processing」的参数不是合法 JSON：EOF whi…`

  `csv-processing` 是 **SkillsBench 的技能**，这条 MCP-Atlas 任务根本不需要它。
  查库确认原因：
  ```
  技能总数 10，模型能看见 10
     ada-plan-view-accessibility / architectural-dxf-extraction / csv-processing /
     geometric-layout-repair / mesh-analysis / pid-controller /
     simulation-metrics / text-parser / vehicle-dynamics / yaml-config
  MCP 服务器：notes  mounted=3
  ```
  这 10 个全是**前几轮跑 SkillsBench 时挂上去的**，`notes` 是更早那轮的
  测试 stub，一路留到了这一轮。
- **为什么不算 quill 的 bug**：quill 里技能与 MCP 服务器是**按用户配置**、
  对该用户的所有对话生效的 —— 用户配一次就一直带着，这是产品设计，
  也正是 ISSUE-007/019 讨论的那个成本问题。**把测试环境搞脏的是 runner**：
  它每条任务只做「挂上本条需要的」，**从不做「撤掉上一条留下的」**。
- **为什么值得记**：它会**双向**污染结论 ——
  1. **撑大输入**：10 个技能的全套正文进了每一条 MCP-Atlas 任务的请求，
     直接放大 ISSUE-019（真实窗口只有 8192）。
  2. **带偏模型**：模型去调一条根本不该出现的技能，于是报出
     「参数不是合法 JSON」这种与本任务无关的错误。
  3. **让批量结果不可比**：第 1 条和第 30 条的运行环境根本不同，
     却会被并排写进同一份 `results.jsonl`。
- **修复方向（未定）**：runner 每条任务跑之前，把当前用户配置收敛到
  **本条任务真正需要的集合**（多余技能停用、多余服务器停用），跑完恢复原状。
  **不要**改成「用完就删」—— 那会毁掉用户真实的配置，
  而这即便是专用测试库也该守住的边界。
- **回归**：跑一批 ≥3 条、每条技能集合不同的任务，断言第 N 条
  `GET /api/extensions/skills` 里 `model_can_see=true` 的集合**恰好**等于
  该任务声明的技能集合。这条断言现在必然失败（实测第 7 条就复现了）。
- **修它之前先撞出的一条真 bug**：见 ISSUE-026 —— `mcp.enabled=false`
  压根不生效，所以「让 runner 能按任务隔离 MCP」这件事在产品侧根本做不到。
- **已修**（2026-10-06，`runner.py`）：
  - 新增 `snapshot_enabled` / `isolate_for` / `restore` 三件套：
    整轮跑之前拍一张启用状态快照 → 每条任务**跑之前**把配置收敛到
    **这条真正需要的集合**（多余技能停用、多余服务器停用）→
    整轮跑完**原样恢复**。刻意**不做**「用完就删」：那会毁掉用户真实的配置，
    这即便是专用测试库也该守住的边界。
  - `run_task` 里**先隔离、再灌本条技能**（顺序反了的话，刚停用的又会被自己挂回去）。
  - 隔离记录随每条结果一起存进 `results.jsonl` 的 `isolation` 字段
    （停用了哪些、有没有收不干净）—— 日后看某条结论时，能知道当时排掉了什么。
  - **收不干净必须喊出来**：停用失败（正文不在磁盘上等）会写进
    `isolation.problem` 并在汇总里显示，那条结果**不能**当成干净环境下的结论。
  - **恢复失败也必须喊出来**：走 stderr 并带「下一步」，不默默咽下去 ——
    留下一份被这轮跑改坏的配置，比报错难查得多。
- **真机验过**（跑 3 条技能集合互不相同的任务，`/tmp/quill-iso.jsonl`）：
  ```
  跑之前  10 个可见：ada-plan-view-accessibility … yaml-config

  [1/3] sb-3d-scan-calc     声明=['mesh-analysis']                          已隔离技能 9 个
  [2/3] sb-bike-rebalance   声明=['geospatial-routing-data', …'scip-opt']    已隔离技能 1 个
  [3/3] sb-citation-check   声明=['citation-management']                      已隔离技能 4 个
       三条的「收不干净」全是：无

  跑之后  10 个可见：ada-plan-view-accessibility … yaml-config   ← 与跑之前完全一致
       "启用状态已恢复成跑之前的样子。"
  ```
  每条停用的个数正好等于「上一条残留的、不属于本条的」那些，逐条对得上。
- **归属更正**：本条一开始被记成「纯 runner 的问题」，这个判断**只对了一半** ——
  MCP 那一半之所以**当时做不了**，是因为 quill 侧 `enabled` 只存不用（ISSUE-026）。
  两条一起修，隔离才真的成立。

---

## ISSUE-027 · 「模型连续 N 轮只调工具不给正文」被报成「模型服务不可用」

**状态**：已修（2026-10-06）
**归属**：**quill 的 bug**，与 ISSUE-018 同一族

- **现象**（实测，`sb-3d-scan-calc` 与 `sb-citation-check` 两条）：
  ```
  provider_unavailable / 模型连续 4 轮都在请求调用工具，没有给出正文。
  已执行的工具：mesh-analysis、list_experts
  ```
- **detail 写得是对的**：「模型连续 N 轮都在请求调用工具，没有给出正文」
  如实说了发生了什么，还带上了已执行的工具轨迹，也给了
  「换个更直接的问法，或检查该工具是否满足不了模型的需求」——
  这几句都没问题，**不许多改**。
- **错的是结构化的 `next_step`**：`api_chat.rs:598` 用的是
  `ApiError::service_unavailable(...)`，而那个变体的 `next_step` 是一条固定串
  「模型服务不可用：先执行 `curl …/models` 确认端点活着；本地模型请先启动
  llama-server…」。于是界面上用户会看到**两句互相打架的下一步**：
  detail 说「换个更直接的问法」，结构化那句说「去把模型服务起起来」。
- **服务明明是好的**：这 4 轮里模型每次都**回话了**（每次都带 tool_calls），
  是模型一直不收敛，不是连不上。这与 ISSUE-018 是同一族 ——
  「连不上」与「回过话了但没给出想要的东西」被同一句话盖住了。
- **与 ISSUE-018 的差别**：`ProviderRejected` 的建议
  （「模型服务**活着**并回了一个错误状态码，它自己的原话在上一段里」）
  在这里**也不合适** —— 什么都没报错，只是没收敛。所以不能直接复用它。
- **修复方向（已做）**：加一个专用的 `ApiError::ToolLoopExhausted { detail, advice }`，
  把「轮次用尽」当成**既不是连不上、也不是被拒**的第三种处境来措辞。
  - `code()` = **`tool_loop_exhausted`** —— 与 `provider_unavailable` /
    `provider_rejected` 都不同，界面才分得开这三件事。
  - `status()` 仍是 **503**（请求确实没拿到正文）。用 502 也许更贴切，
    但那会改对外状态码，**这一轮不做**，另议。
  - `advice` 是 `error.rs` 里的 `ADVICE_TOOL_LOOP_EXHAUSTED`，
    **与 detail 里那句「换个更直接的问法」说同一件事**，并额外点名
    「先停用这一轮挂着的技能/工具：它们可能让模型觉得还得再查一下」——
    这条是从 ISSUE-019/025 攒下来的经验，不是编的。
  - `api_chat` 那一处从 `service_unavailable` 改为 `tool_loop_exhausted`。
    **detail 一字未改**（它本来就写得对）。
- **回归**：
  - `error::tests::a_model_that_never_stops_calling_tools_is_not_an_unreachable_endpoint`
    —— 断言 `code == "tool_loop_exhausted"`、与另两个 provider 码**都不同**；
    `next_step` 里**不得**出现「确认端点活着 / 启动 llama-server /
    回了一个错误状态码」；且**必须**与 detail 同向（含「更直接」）。
  - `error::tests::every_variant_carries_a_non_empty_next_step`
    顺带覆盖了新变体（枚举里新增的每个都要有非空下一步）。
  - `error::tests::a_live_model_that_refuses_must_not_be_reported_as_unreachable`
    未受影响（ISSUE-018 那条仍然成立）。
- **注**：4B 小模型在工具上打转**本身**不算 bug（红线里写了「4B 答不上来不算 bug」），
  算 bug 的是**附带的处置建议指错了地方**。这条修的是后者。

---

## ISSUE-028 · 客户端等不下去被记成 `FAIL`，而 `FAIL` 是计入跑过的

**状态**：已修（2026-10-06）
**归属**：**测试脚手架（`runner.py`）的缺陷，不是 quill 的**

- **现象**（50 条批量跑时撞见，`sb-crystallographic-wyckoff-position-analysis`）：
  ```
  FAIL | err=transport / timed out
  ```
- **根因是一笔算得出来的账**：
  ```
  quill 单次模型调用超时 DEFAULT_TIMEOUT_SECS = 300s   （llm.rs:104）
  一轮对话最多 1 + MAX_TOOL_ROUNDS 次串行调用          （MAX_TOOL_ROUNDS = 4）
  → 一轮最坏能到 5 × 300 = 1500s
  runner 单请求超时原来是 180s
  ```
  180 远小于 1500。所以「超时」表达的是「**我等不了了**」，
  **不是**「quill 出问题了」。
- **为什么严重**：runner 把 `__http == 0` 一律归到「链路不通」，
  `verdict()` 于是给 `FAIL`；而 `FAIL` 按定义是
  「链路不通，或命中 quill 自己的 bug」，并且**计入
  `PASS + PARTIAL + FAIL` 的「真的跑完」**。
  也就是说 —— **我自己等不下去，会被算成「quill 跑完了、只是没通过」**。
  这正是红线里最不许出现的那种虚增，而且是**系统性**的：
  工具循环越多、单轮越慢，命中率越高。
- **已修**：
  - 新增判定 `TIMEOUT`：**客户端自己等不下去了**，结果没拿到。
    与 `UNJUDGEABLE` 一样**不计入**「真的跑完」，但含义不同 ——
    `UNJUDGEABLE` 是「跑完了、验不了」，`TIMEOUT` 是「压根没拿到结果」。
  - `judge()` 识别 `transport` + `timed out` 这一组合，置 `timed_out=True`，
    并把 `link.why` 改成「客户端超时，没拿到结果（这不是 quill 的错）」——
    **错误原文照旧带出去，不改写**（改写过的错误没法用来排查）。
  - runner 默认超时 180s → **600s**，并加 `--timeout` /
    `QUILL_RUNNER_TIMEOUT` 可调；`--help` 里写明它只是**下限**，
    因为 quill 最坏能到 1500s。
- **回归**（`.wsl-t28.sh`，4 条断言，**已验证不是空断言**）：
  - 超时 → `TIMEOUT`，且**不等于** `FAIL`
  - 真·链路不通 → **仍然** `FAIL`（别把这个洞修过头）
  - 链路通但没维度可判 → `UNJUDGEABLE`（ISSUE-017 那条不许被带坏）
  - 默认超时 ≥ 600s
  - 负向验证：把 `res.get("timed_out")` 那个分支改掉之后，
    第 1 条**立刻红**（`AssertionError: 超时应判 TIMEOUT，实际 UNJUDGEABLE`），
    改回后恢复。
- **没有单测进仓库门禁**：本仓库的 Python 不在 `.wsl-verify-persona.sh` 里，
  上面 4 条是本轮自用脚本。这与 `runner.py` / `build_tasks.py` 的现状一致，
  **如实记着，不假装它有门禁保护**。

---

## ISSUE-026 · MCP 服务器的 `enabled=false` 只存不用：用户停用了，工具照样挂进对话

**状态**：已修（2026-10-06）

- **严重度**：中等偏高（又是「界面/配置说一套、实际做另一套」，
  与 ISSUE-008 `tool_allowlist` 只存不用是同一类病）
- **现象**：`mcp_repo` 里有 `enabled` 列，`POST /api/extensions/mcp` 也从
  请求体读它（`parse_server`：`obj.get("enabled")…unwrap_or(true)`），
  `mcp_body` 也把它回给前端。但 `with_mcp_tools` 里：

  ```rust
  let rows = crate::mcp_repo::list(db, uid).await?;
  let found = mcp_client::discover_all(rows.clone()).await;   // ← 一行都没判 enabled
  ```

  **用户把服务器停用了，这一轮照样握手、照样把它的工具挂进对话工具表。**
  界面上没有任何提示，模型也照调不误。
- **对比**：同一条路径上的**技能**是尊重 `enabled` 的
  （`skill_visibility` 里 `if !row.enabled { return Disabled }`，
  `with_skills` 里 `Disabled => continue`）。所以这不是「设计上就没这个概念」，
  是**两条路径待遇不一致**，MCP 这条漏了。
- **已修**（两处必须一起改，否则会**制造一个新的谎**）：
  1. `tools::with_mcp_tools` 过滤掉 `enabled=false` 的行，全停用时直接返回，
     **不去握手** —— 否则等于把用户主动关掉的进程全拉起来一遍。
  2. `api_extensions::mcp_body` 用**同一个** `tools::enabled_servers` 过滤。
     **只改第 1 处会造出「界面说 mounted=3、模型一个都调不到」** ——
     那正是 ISSUE-014 那一类、两处口径不一致的谎。
     停用的那台在 `status` 里**如实出现**（`probed:false` / `connected:false` /
     `mounted:0` / `disabled:true` + 中文「下一步」），**不抹掉** ——
     抹掉的话用户会以为它被删了。
     `servers` 仍返回**全部**行（含停用的），否则前端回填编辑表单时开不回来。
- **回归**：
  - `tools::tests::a_disabled_mcp_server_is_not_mounted_at_all`
  - `tools::tests::every_server_disabled_leaves_nothing_to_mount`
    —— 全停用时**不许再去握手**
  - `tools::tests::no_server_configured_stays_empty`
  - 过滤逻辑抽成自由函数 `tools::enabled_servers` 才测得着：
    `with_mcp_tools` 本体要真起 stdio 进程，测不了「过滤」这一层。
  - 全量 **966 passed / 0 failed**（改前 963）。
- **真机验过**（重启服务 → POST `enabled:false` → 再查）：
  ```
  停用前  notes  probed=True  connected=True  mounted=3  disabled=None
  停用后  notes  probed=False connected=False mounted=0  disabled=True
          error= 这台服务器已被停用：这一轮不握手、也不会挂任何工具进对话。
                 下一步：要重新启用，在编辑里把「启用」打开并保存。
          servers 里仍然列出了这台（否则用户在界面上开不回来）: ['notes']
  ```
  验完已把 `enabled` **恢复成 true**（`mounted=3`），没给下一轮留改脏的配置。
- **未修（别当成已修）**：
  1. **前端没有开关**。`ui/web/src/devices/api.ts` 的 `McpServerConfig`
     **没有 `enabled` 字段** —— 界面只有 `enabled_capabilities`
     （那是「启用哪些能力」，不是「这台跑不跑」）。所以这条修好之后，
     用户仍然**没法在界面上停用一台 MCP 服务器**，只能走 API。
     `api.ts` 的注释其实已经预期到了：「停用 / 非 stdio / 缺 command 都会是 false」，
     说明这个状态被设计过，只是没有入口。
  2. ~~**`runner.py` 的任务隔离仍未做**~~ → **已修，见 ISSUE-025。**

---

## ISSUE-029 · 一个专家都没有，`list_experts` 仍然每条请求都挂着 —— 4B 在上面死循环

**状态**：**已修 + 单测已补 + 门禁已过 + 真机复跑已验有效**

- **严重度**：中等偏高（它是 50 条批量跑里**失败最多的单一原因**）
- **现象**（实测，50 条 SkillsBench **全部跑完**后的统计）：
  | 指标 | 数量 |
  |---|---|
  | 总条数 | 50 |
  | `FAIL` | 43 |
  | `UNJUDGEABLE` | 7 |
  | `PASS` / `PARTIAL` | **0 / 0** |
  | 失败原因为「连续 N 轮只调工具不给正文」 | **34** |
  | **轨迹里出现 `list_experts` / `get_expert_detail`** | **28** |
  | 其中**同时**是工具轮次耗尽 | **28**（100% 重合） |
  | 上下文超窗（上游 400 `n_ctx`） | 5 |

  典型轨迹：`sb-citation-check` —— `list_experts、list_experts、list_experts、get_expert_detail`，
  4 轮预算一次没花在正事上；`sb-flink-query` —— `list_experts、get_expert_detail、list_experts、list_experts`。
- **根因**（查 `GET /api/experts` 得到的事实）：
  ```
  {"experts":[]}
  ```
  这台库上**一个专家都没有**。于是：
  - `list_experts` 每一条请求都挂在工具表里，而它**永远只能返回
    「没有匹配的专家。」**；
  - `get_expert_detail` 同样挂着，也**必然失败**。

  模型在这两个必然空手的工具上反复重试，直到 `MAX_TOOL_ROUNDS` 用尽，
  于是一条本来能答的任务被判成「连续 4 轮只调工具不给正文」。
- **为什么算 quill 的问题而不只是「4B 笨」**：红线里「4B 答不上来不算 bug」
  针对的是**答案质量**。这里是**工具表里放了两个对当前用户必然产不出东西的
  工具** —— 既占 token（ISSUE-007/019 那一类成本），又把模型往一个空手的
  分支上引。给一个没有专家的用户挂专家工具，是纯粹的噪音。
  数字上：**28/50 = 56% 的任务**的工具预算被这两个空工具吃掉。
- **已修**：`ToolRegistry::builtin` 挂载前先查一次
  `api_experts::list_for_tools`（**与工具内部用的是同一条可见性路径**，
  不另开一条后门），**列表为空就不挂这两个工具**。
  - **查库失败时按「有专家」挂上**，并记一行日志：那是 quill 自己的存储出问题，
    静默少挂两个工具会变成「模型好像没学过专家」且毫无迹象 ——
    与 `with_skills` 对查库失败的处理是同一个取舍。
- **回归**：
  - `cargo test -p quill-server --lib tools::tests` **18 passed / 0 failed**
    （含 `a_builtin_registry_is_not_empty_and_every_spec_has_a_handler` ——
    提醒一句：那条断言的是「内置表非空」，**不是**「专家工具必须在」，
    所以这次改动不该动它）。
  - `cargo test -p quill-server --test extensions_http` **44 passed / 0 failed**。
  - **新补的用例**：`no_experts_means_no_expert_tools_in_the_chat_tool_table`，
    一条里钉两件事 ——
    1. 专家数 0 时工具表里**没有** `list_experts` / `get_expert_detail`；
    2. 种一个专家进去后**必须挂回来**（防「一刀切地不挂」）。

    上一轮记的「这条新判断没有单测保护」到此作废：`builtin()` 确实需要
    `Arc<AppState>` 与真实 DB，纯函数单测构造不出来 —— 但 `extensions_http.rs`
    的 `Harness` 本来就有真实 DB 和真实 router，**正确的落点在那里，不是 tools 单测**。
- **回归中撞到并修掉的一件事**（值得单独记，它会让「改测试」变成「改对测试」）：
  - 这次改动让 `extensions_http.rs` 里 **3 条用例变红**：
    `a_dash_named_skill_coexists_with_the_underscore_named_builtin`、
    `an_enabled_skill_reaches_the_model_as_a_tool_and_can_be_called`、
    `mcp_tools_from_a_real_server_reach_the_conversation_tool_table`。
  - 它们红的原因**不是产品有问题**，而是拿 `list_experts` 当「内置工具还在」
    的探针，而它们的 fixture 专家数是 0。
  - **修法不是把断言删掉**：先在 fixture 里 `seed_expert(&h, "cost-analyst").await`
    把「用户确实有专家」这个前提补上，断言原样保留。这样这 3 条测的仍是
    它们本来要测的东西（SKILL / MCP 不挤掉内置工具），
    「专家数为 0 时不挂」由新用例单独负责。
  - ⚠ 踩坑记录：写 `seed_expert` 时**漏了 `.await`**。async fn 不 await
    **不会编译报错**，只有一条 `unused_must_use` 警告，症状是「专家没种进去、
    后面的断言莫名其妙地红」，一度以为是真 bug。已把这条写进 helper 的注释里。
    现在 `extensions_http` 编译输出里 `futures do nothing unless` 计数 = **0**。
- **真机复跑（已做，结论：修复有效）**：
  50 条批量跑完、服务空闲后，用**新二进制**重启 quill-server，
  再复跑两条**上一轮 4 轮预算全烧在专家工具上**的任务。对照如下：

  | 任务 | 修前（已执行的工具） | 修后（已执行的工具） |
  |---|---|---|
  | `sb-flink-query` | `list_experts、get_expert_detail、list_experts、list_experts` | `pdf、pdf、pdf、pdf` |
  | `sb-citation-check` | `list_experts、list_experts、list_experts、get_expert_detail` | 不再报工具轮次耗尽（判为 UNJUDGEABLE） |

  - 专家工具**一次都没再出现** —— 专家数为 0 时它们真的不挂。
  - `sb-flink-query` 仍然 FAIL，但**原因换了**：现在烧预算的是 `pdf` 这个
    真实技能工具，不是空转的专家工具。按红线「4B 答不上来不算 bug」，
    这一条剩下的不是 quill 的 bug。
  - 顺带看到 ISSUE-027/028 生效：错误码从 `provider_unavailable`
    变成了 `tool_loop_exhausted`，两类故障不再混报。
- **负向验证**（不看「测试通过」，得看它会不会红）：
  临时把 `Ok(list) => !list.is_empty()` 改成 `Ok(_list) => true`（回到修复前行为），
  `no_experts_means_no_expert_tools_in_the_chat_tool_table` 如期变红：
  ```
  一个专家都没有却挂了专家工具，它们只能永远空手：["list_experts", "get_expert_detail"]
  ```
  破坏与回滚都在脚本内完成，`tools.rs` 已确认复原（`grep` 确认第 147 行仍是
  `Ok(list) => !list.is_empty()`）。随后又把那个 `if has_experts {` 块的缩进
  用 `rustfmt` 补齐 —— 被删掉的行逐行核对过，全是这个块里没缩进的 `register`，
  没有顺手改到别的地方。

---

