# 刻意偏离上游的地方（queue Q079）

> **这份文档回答一个问题**：哪些地方我们**知道 goose / octop 怎么做，却故意不做成那样**？
>
> 它和 [`UPSTREAM-USAGE.md`](../UPSTREAM-USAGE.md) 是一对：那份说「我们用了上游的什么」，
> 这份说「我们**没**跟上游的什么、为什么」。`node .provenance-check.mjs` 核的是前者
> 那些 `file:line` 引用；这份清单里的每一条也要能回上游核对。
>
> **纪律**：每一条写清三件事 —— ① 上游怎么做（带 `file:line`）、② quill 怎么做、
> ③ 为什么（以及**代价是什么**）。只写「我们不一样」而不写代价，等于把缺口包装成设计。
>
> 核对日期 2026-10-09。凡与代码不符的，以代码为准并回来改这份文档。

---

## A. 内核侧（对 goose）

### A1. 对话循环：自研，不逐行抄 `Agent::reply`

**上游**：`vendor/goose/crates/goose/src/agents/agent.rs::reply` / `reply_impl`
（`:2079` 起）是 agent 的一轮主循环；工具往返在 `agents/tool_execution.rs`。

**quill**：`crates/quill-core/src/turn.rs::run_turn`（Q012 从 `api_chat.rs` 搬入），
**自创**：取历史 → 建请求 → 流式或一次性 → 有工具就执行回灌 → 直到正文或预算用尽。

**为什么**：goose 的 `reply_impl` 里混着**会话存储、权限确认（`tool_confirmation_*`）、
状态机、telemetry、extension manager** —— 在 quill 那些是别处的活（落库在壳、状态机是
另一个模块且未接线）。照抄会把五件事一起搬进来，而其中四件在 quill 还没有对应物。

**代价**：`turn.rs` 的行为细节（比如流式失败退回一次性）是 quill 自己定的，**不能**
拿 goose 的测试当证据。已如实标注：`turn.rs` 头注明「自创 + 对照上游，不是抄自上游」。

### A2. 上下文压缩：逻辑照抄，**删工具响应的重试阶梯不触发**

**上游**：`goose-context-management/src/summarize.rs:121-182` 的 `summarize` ——
摘要器自己溢出（`ProviderError::ContextLengthExceeded`）时按
`REMOVAL_PERCENTAGES = [0, 10, 20, 50, 100]` 逐级删工具响应重试。

**quill**：阶梯、百分比、文案逐字照抄在 `quill-core/src/compaction.rs`，
但壳侧（`quill-server/src/chat_compaction.rs`）**不产生 `ContextLengthExceeded`** ——
它把上游错误一律归到 `CompactionError::Model`。

**为什么**：`quill-provider::ProviderError` 没有「上下文超了」这个分类，
照 body 里的关键字去猜就是替上游说话（这条判断在 `error.rs` 的
`ADVICE_REJECTED_BY_STATUS` 注释里有先例）。宁可让阶梯**不触发**，也不猜错。

**代价（明写）**：quill 的压缩在「摘要请求本身超窗口」时**直接失败**（响应里带 `note`），
不会像 goose 那样删掉旧工具响应再试。要补得先有一个可信的分类器。

### A3. token 估算：启发式，不是真分词器

**上游**：goose 的 `TokenEstimator` 异步加载 tiktoken 实现
（`goose/src/context_mgmt/mod.rs:305-313`），`estimator` 是 `Option`。

**quill**：`quill-server/src/chat_compaction.rs::CharHeuristic` —— 纯函数，
ASCII 4 字符 1 token、非 ASCII 1 字符 1 token；内核侧 `TokenEstimator` 是
**必填**且同步（`compaction.rs` 模块头第 2 条写了差异与理由）。

**为什么**：现有依赖里没有分词器（不新增依赖），而压缩前后两本用量账必须能对上，
所以估算器不能是 `Option`。不用「一律 chars/4」是因为中文会被低报约 4 倍，
阈值永远打不到 —— 那等于没接。

**代价**：响应里的数字是**估算**（字段名就叫 `estimated_history_tokens`，界面照这个说法显示）。
真要精确得引一个分词器，属新依赖，需要先说清「为什么现有依赖不够」。

### A4. 落库留在壳，不引入 `TurnRecorder` 端口

**上游**：goose 的 agent 循环自己写会话（`Agent::reply` 一路会更新 session）。

**quill**：内核 `turn::run_turn` 返回 `TurnOutcome`，**不写库**；壳拿到结果后落库
（`api_chat::finish_turn`）。设计原文在 `docs/KERNEL-PORTS.md §4.1`：
「先不引入 `TurnRecorder` 端口 —— 返回值就够，等真需要再加，别为对称而对称」。

**代价**：内核在循环中途**无法**落库（比如每轮工具结果都想存一份就得改端口）。
目前不需要。

### A5. 会话模型：一张表 + 一问一答，不是可重放的 message 流

**上游**：goose 的 session 是「可重放的完整 message 流 + 元数据」，工具往返也是消息，
可见性在 message metadata，状态由消息尾部推导。

**quill**：`sessions` + `messages` 两张表，工具往返**不落库**，`sessions.state` 插入时
写死 `'IDLE'` 后再没动过。逐条对照见 `docs/KERNEL-ALIGNMENT.md §Q030`
（12 条维度：等价 0 / 部分 5 / 缺失或语义相反 7）。

**为什么**：这是 quill 的历史起点（`api_chat` 自研），对齐它是**独立的大活**，
不是「照抄一段代码」。Q030 给出第一步落点（让工具往返入 `messages`）。

**代价**：界面上看不到工具往返的存档；`error_state`/`replan_count`/`summary*`/
`compacted_count`/`checkpoint_*` 这些列在 Rust 侧**零读写**（Q030 记录在案）。

---

### A6. 记忆（memory）：逻辑照抄，两条传输都在，**默认不预置**

**上游**：goose 把记忆做成一个**内置 MCP 扩展** —— `goose-mcp/src/lib.rs` 的
`BUILTIN_EXTENSIONS` 里 `memory` 一项，四个工具
（`remember_memory` / `retrieve_memories` / `remove_memory_category` /
`remove_specific_memory`），背后是按分类落盘的 `.txt`。它有一条**进程内**路径
（`vendor/goose/crates/goose/src/agents/extension_manager/builtin.rs:35-40` 用
`tokio::io::duplex` 起服务器），也有一条 **stdio** 路径
（`goose mcp memory`，`vendor/goose/crates/goose-mcp/src/mcp_server_runner.rs:36-49`）。

**quill**：`crates/quill-core/src/memory.rs` 照抄了那套存储语义与四个工具
（含分类名的安全边界、全局记忆拼进 instructions）。**两条传输也都有**：stdio 入口是
`quill mcp memory`（`crates/quill-cli/src/main.rs` 的 `run_mcp`），进程内那条是
`serve_on_duplex`，对上配置行的 `transport='builtin'`（`crates/quill-core/src/builtin.rs`，
queue Q111）。**接进对话 = 登记一行配置**（`transport='builtin'` + `command=memory`），
保存即真的握手，四个工具进那一轮的工具表、模型调得到。

**代价 / 还没做的**：
- **默认不预置**：quill 不往任何用户的配置里塞这一行 —— 装完没有记忆，用户要自己
  去设备页加一行才有。「默认给谁开」是产品决定，不是技术缺口。
- 本地目录是 `.quill/memory`（上游 `.goose/memory`）；全局目录用
  `default_global_memory_dir()` 现算，**没引 goose 用的 `etcetera`**。
- `remove_specific_memory` 的删除口径是「内容**包含**」而不是「相等」（照抄上游，
  未收紧）—— 给一段更短的子串会连带删掉所有含它的条目。

---

### A7. 会话回滚：照 goose 做**原地删除**，没做 octop 那条**非破坏性 fork**

**上游**（两个来源，形状不同）：
- **goose**（内核）：`session/session_manager.rs:2631-2660` 的
  `truncate_conversation_from_message` —— 给定一条消息，把它**连同之后**的全部
  **原地删掉**（`>= 边界`）；另有 `:2575` 的 `copy_session`（复制一个会话）与
  `:2620` 的 `truncate_conversation(session_id, timestamp)`。
- **octop**（外壳）：`POST /agents/{agent_id}/threads/{thread_id}/fork`
  （`summary="Fork thread from an assistant message"`，`.octop-ref/octop/src/octop/api/routers/chat/history.py:419-446`）
  —— **不动原线程**，另建一个新线程、把历史复制到选中的那条 assistant 消息为止，
  让用户从那里续写。定位参数 `ForkThreadBody` 支持 `message_id` / `content` /
  `assistant_turns_from_end`（`.../chat/models.py:152-167`）。

**quill**：`crates/quill-server/src/chat_repo.rs::rollback_from_message` +
`POST /api/sessions/{id}/rollback`（`api_chat::rollback`）—— 走的是 **goose 那条**：
原地删除边界消息及之后的一切，登记在 `EXTRA_ROUTES`。

**为什么选 goose 那条**：Q021 的判据是「**能回滚到某一轮**」，goose 的 truncate 正是这个；
octop 的 fork 是**分支**不是回滚。而且 **octop 的 fork 实现在 sparse 集合之外**
（`src/octop/infra/agents/threads/` 不在检出里，`fork_dashboard_thread` 读不到）——
照抄不了，硬做就是自创（违反第 3 条）。所以先落地读得到的那条。

**代价 / 还没做的**：
- **没有前端入口**（没有「回滚到这里」按钮）。
- **fork 没做**：非破坏性那条（`copy_session` + 可选 truncate）在 goose 里读得到、
  octop 的路由形状也读得到，但 octop 的实现读不到 → 要做先拍板（quill 侧是
  另建会话，还是要不要把 `sessions` 的一堆列也复制过去）。
- **语义不同要认清**：quill 这条是**破坏性**的（消息真被删）；octop 那条不会动原线程。
  用户从一个产品换到另一个，行为不一样 —— 这是有意的，写在这里以免被当成 bug。

---

### A8. 运行中成员的控制面：**形照抄，取消手段换了一种**（queue Q023/Q024）

**上游**：goose 有一张**进程级注册表**把「正在跑的那个」与它的取消令牌放在一起 ——
`vendor/goose/crates/goose/src/execution/active_run.rs:1-14` 的
`ActiveRun { run_id, cancel_token, agent }`（字段注释写着「Routes steering from another
roaming connection to the run owner」）与 `:103-115` 的 `agent_cancel_token` /
`cancel_agent_run`；追加指令走**每会话一个 FIFO**，由状态机在**轮与轮之间**注入
（`vendor/goose/crates/goose/src/agents/agent.rs:562-600`、
`vendor/goose/crates/goose/src/agents/state_machine/ops_steer.rs:1-70`）。

**quill**：`crates/quill-server/src/member_control.rs` 照抄了这张注册表的形
（登记 → 另一个请求找得到 → 取消 → 跑完摘掉），键换成 **(发起人, 成员)**：
一轮派工里所有成员共用 leader 的 session，按 session 键会把同轮成员串在一起。

**刻意不同的两处（各带代价）**：
1. **取消不用 `tokio_util::sync::CancellationToken`，改用 `tokio::sync::Notify` +
   `select!`** —— 代价：`notify_one` 的「没人在等时存一张通行证」语义得自己讲清楚
   （`member_control.rs` 里写了注释并有用例钉住 `a_cancel_that_arrives_before_the_wait_still_wins`），
   而 `CancellationToken` 天生就有这个语义。换来的是一条新依赖不进门。
2. **没有在跑的成员时如实报错**（`AdapterError::NotFound` + 下一步），goose 那边
   队列可以存在但没有消费者 —— 我们不接受「假装送达」，也不排队等下一次
   （那意味着一次调用可能永远不生效，而调用方以为自己成功送达了）。

**HTTP 出口（Q113，2026-10-09 已接）**：`POST /api/dispatch/{member}/steer` 与
`/abort` 两条路由对**另一个请求**里正在跑的成员生效 —— 注册表因此从「执行器私有」
搬到 `AppState` 上共享（`api_dispatch::run` 建执行器时 `with_control` 接上；
执行器仍是每请求现建，但注册表不再跟着每请求一份）。**上游没有对应形状**
（goose 的 steer 走「roaming connection」那条道，见上面 `active_run.rs` 的字段注释），
所以**路由形状（路径 / 请求体 / 错误码）是自创的**：没在跑的成员 → 404、
已取消的成员 → 409、标识形状不对 → 400；`AbortScope` 只暴露 `StopRound`
（`AbortRoom` 留在执行器层 —— 不给没有需求的参数）。

---

## B. 外壳侧（对 octop）

### B1. 团队派工会话（`kind='team_leader'`）：**自创**，octop 没有这一层

**上游**：octop 的 team 只是**专家名册**；goose **完全没有多智能体**。

**quill**：`POST /api/teams` 顺带插一条 `kind='team_leader'` 的会话作为派工记账落点
（`teams.leader_session_id` NOT NULL + 外键 + `POST /api/dispatch` 强制要它）。
建它的是 `quill-server/src/teams_repo.rs` 的 `INSERT_LEADER_SESSION_SQL`（不是 api_teams）。

**为什么**：派工要有地方记账，而 quill 的会话表是现成的记账单位。

**代价**：那条会话会出现在用户侧栏里（前端不认识 `team_leader` 这个 kind），
所以 `GET /api/sessions` 加了 `exclude_kind` 过滤 —— 过滤放后端是因为那条会话
跑起来之后**真的有消息**，前端藏起来会让侧栏计数和实际数量对不上。
跟踪项：queue Q052（「建团队时静默多出的那个会话，不在侧栏露出来」）。

**Q025 把同一层再往下延了一级**：派工真跑一轮时，每个成员各有一条自己的
`kind='team_member'` 会话（`chat_repo::INSERT_MEMBER_SESSION_SQL`；
`api_dispatch::ensure_member_sessions` 在记账/执行前**缺则建**，`parent_session_id`
指回主持人会话），成员的任务与产出作为一对 user/assistant 消息写进那条会话
（`persist_member_outputs`）。**「每子 agent 独立会话」这件事本身是对齐 goose 的**
（`vendor/goose/crates/goose/src/agents/subagent_handler.rs:178-190`：每个子 agent 一个
`session_id`，经 `SessionManager` 落盘）——quill 自己的是**存法**：用户对话、主持人、
成员的会话挤在同一张 `sessions` 表里，靠 `kind` + `parent_session_id` 分家
（schema 的 CHECK 对 `team_member` 强制三项归属非空，`0001_init.sql:284`）。
所以成员会话也要靠 `exclude_kind` 才不混进侧栏（前端发的是
`?exclude_kind=team_leader,team_member`）。
**代价**：① 成员会话在「全量会话」口径里可见（团队页/用量页看得见它）；
成员这一轮的 token 用量**已记**（Q026：执行器跨轮累加 `MemberUsage` → 随产出
写进那条会话的 assistant 消息，列可空，上游没报就是 `null` 而非 0；用量页因此
看得到派工消耗）；② 成员会话按调用方给的 id 复用，不回收，
`message_count` 随派工轮数增长，会话数会被「每成员一条」放大。

### B2. 通道（channels）：只做微信一条，其余诚实降级

**上游**：octop 的通道（`channels.py`，21 条路由）支持九种平台（微信 / 钉钉 /
企业微信 / 飞书 / 元宝…，各有一套 qrcode 与 bot-creator），并有 `/{id}/test`
（探测已保存的通道）与 `/probe`（探测未保存的草稿配置）。

**quill**：只做**微信**一条（用户明确要求「连接一个微信就可以」）。
`crates/quill-server/src/api_channels.rs` 的路由**形状照 octop**
（list/create/get/delete + `/{id}/test` + weixin 的 qrcode generate/poll；
quill 去掉 agent 前缀 —— agent 就是登录用户自己），但：
- **其余四个平台不做**（Q045 已拍板「先不做其它平台」）。它们的集成代码在
  octop 的 sparse 检出之外，做就是自创；按本仓纪律不画假入口。
- **`/probe` 不做**：微信的凭据来自扫码（没有「草稿配置」可探），
  它在本仓没有落点。
- **`/{id}/test` 已接**（Q045），且**只说自己能说的那件事**：它证明
  「端点可达 + 已配置凭据」，**不假装验证了「登录仍有效」** —— 后者要发起一次
  真实收发，会抢走正在轮询的消息（本模块头写明的协议限制：一个凭据只能有一个
  长轮询者）。所以探测走 `get_bot_qrcode`（无副作用、不碰游标），
  结论与「没验证的那一半」都写进响应。

**为什么**：这是「先有真后端再放按钮」的纪律（最高指示第 2/5 条），
宁可页面上少一个开关，也不给一个点了必失败的东西。

**代价**：① 钉死/企微/飞书/元宝四家的用户在本仓没有通道可用（与 octop 的能力差
是有意的）；② `/test` 的 `ok:true` 不等于「登录有效」，用户看到绿灯仍可能在某次
真实收发上碰到 `-14 会话过期` —— 响应里的 `note` 与前端那句话都点明了这一点，
不把它藏起来。

### B3. 资料库的写入面（`PUT/DELETE /api/wiki/pages/{path}`）：**自创**，两边都没有

**上游怎么做**：`.octop-ref/octop` 的知识库是 **embedding/RAG** 那套
（`src/octop/api/routers/knowledge_bases.py`，961 行：上传 / OCR / 向量 / 检索），
**没有「页面」这一层**，也没有「人直接编辑知识库页面」的入口；goose 侧根本没有知识库概念。
quill 这套资料库（raw/wiki/schema 三层 + `index.md` + 变更日志）来自用户自己的
**xu-wiki**，而它**不在本地**（`vendor/` 只有 goose 与 openoctopus-frontend）——
所以「页面怎么读写」这件事**没有可抄的源**。

**quill 怎么做**（`crates/quill-server/src/api_wiki.rs`，2026-10-09 / Q058）：
- 版本号 = **磁盘上那份原文的 sha256**（`page_version`），不是 `parse_page` + `render_page`
  之后重排的形状 —— 后者会让「用户手上那版」与「库里那版」算出的版本对不上；
- `expected_version` **缺省或 null** = 「这一页必须还不存在」（新建）；给值 = 「我看到的就是这一版」；
- 删除**必须**带版本号（缺 → 400），因为删除不可逆，不带版本的删除会删掉别人刚写的那版；
- 写入前先用 `page_from_wire` 校验内容能解析成合法页面，否则 400 —— 不许把坏页写进库
  （坏页会进 `index.md`，而下次 ingest 会把它当既有页）。

**为什么自创**：没有可抄的源（见上），而这是产品里必须有的一条路 —— 前端
`ui/web/src/memory/api.ts` 的 `WIKI_WRITE_ROUTE` 早就把 `PUT /api/wiki/pages/{path}`
这条契约写死了，后端一直没接（2026-10-09 才接上）。

**代价（如实记）**：① 版本号是**内容哈希**，不是「第几版」——历史不可追溯（要历史得另做）；
② 与 ingest 的关系未定：模型摄入写出的页也进同一目录，版本号同样由这个函数算，
所以两边不会打架，但「谁改的」只能从变更日志读；
③ `expected_version` 是**我们自己起的名字**（不是 `ETag`/`If-Match`），
而工作区页（Q059）说的是「ETag 乐观并发」—— 两套说法，真做工作区时要统一。

### B4. 资料库的模型侧协议（`POST /api/wiki/{ingest,query}`）：**自创的 JSON 形状**

**上游怎么做**：xu-wiki（用户自己的项目，公开仓库）的架构是「**CLI 只做确定性的活、
CLI 不调 LLM**，内容由 agent 决定」（`design-docs/06-query.md` 的 `[PRIN-QRY-3]`；
`design-docs/05-ingest.md` 的 `[PRIN-ING-1]`「commit 是唯一写盘入口」）。那边**没有**
「模型输出什么 JSON」这回事 —— agent 直接调 CLI 命令。

**quill 怎么做**（`crates/quill-server/src/wiki_backend.rs`，2026-10-09 / Q057）：
quill 里那个「agent」就是服务端自己，于是把上游的两段分工映射成
**模型产出正文 / 答案 → 壳校验并写盘 → `quill-wiki` 重建索引、写日志**。
为了让结果可判定，要求模型回 JSON：摄入 `{"pages":[{"path":…,"content":…}],"summary":…}`、
问答 `{"answer":…,"citations":[…]}`。

**为什么自创**：上游没有这个形状（它是 agent 驱动 CLI），而 quill 是常驻服务，
必须自己把「模型输出」定成机器可判定的东西。

**代价（如实记）**：① 这是**quill 的壳内约定**，换个模型就可能不遵守 —— 所以解析失败、
页面不合法一律**报错**，绝不静默降级成「这次摄入什么都没做」（那是最坏的结果：
用户以为存进去了）；② 与 xu-wiki 的**两阶段**（解析暂存 → commit）不同，quill 是一步
（模型直接产出成品页），没有暂存层，因此上游那套「解析器插件 / SHA256 三路去重 /
300 行切分」在 quill 侧**完全没有**（差集表见 Q056）；③ 源文必须先放进 raw 层，
摄入端点只收**路径**不收正文 —— 否则「摄入」就成了无出处的写入。

### B5. 定时任务：**只做间隔与指定时刻，不做 cron 表达式**（`/api/cron`）

**上游怎么做**：octop 有整套定时任务（`.octop-ref/octop/src/octop/api/routers/cron.py`，
255 行：list / create / get / patch / delete / run-now，排期交给它的调度器）。

**quill 怎么做**（2026-10-09 / Q042，`crates/quill-server/src/{api_cron,cron_repo,cron_scheduler}.rs`）：
路由形状照 octop（去掉 `/agents/{agent_id}` 前缀 —— agent 就是登录用户自己），
但排期**只支持两种**：`every`（固定间隔秒）与 `at`（指定时刻，一次性，投递后软删）。
`cron_expr` + IANA 时区**明确拒绝**（400 + 下一步），并且**表单里那个选项也去掉了**。

**为什么**：cron 表达式 + IANA 时区需要 cron 解析与时区库（本仓依赖树里只有 `chrono`，
没有 `cron`/`chrono-tz`），而 **DST 感知的时区算错一小时是用户可见的错** ——
「工作日九点」变成「工作日八点」比没有这个功能更坏。宁可不做，也不做错。

**代价（如实记）**：① 做不到「工作日九点」这类排期 —— 用户只能用「每 86400 秒」近似，
而那在跨 DST 时会漂一小时；② 前端**少一个选项**（不画做不到的入口），与 octop 的能力面
有差距，这条差距是**有意**的；③ 要补的话，加 `cron` + `chrono-tz` 两个依赖并把
`parse_schedule` 的 `"cron"` 分支从「拒绝」改成「真解析」即可 —— 落点很小，等真需要再做。

---

## C. 工程侧（对两边都不抄）

### C1. 依赖分层守卫是自己写的（`.layer-guard.mjs`）

**上游**：goose 用 workspace 约定 + 人的自觉；octop 是 Python，没有对应物。

**quill**：`.layer-guard.mjs`（依赖只能向下 + 基线清单：新违例报红、陈旧条目也报红，
带 `--self-test`）。为什么用文本脚本而不是 `cargo-deny`/`cargo-machete`：
那些**不知道 quill 自己的层次编号**（L0–L4）。

**代价**：它是**文本级**检查（读 `[dependencies]` 段），不是依赖图真值 ——
`[dependencies.x]` 子表、`target.*` 这些都单独处理过，但仍然是启发式。
另在 CI 里有「每个门禁都必须有非 0 退出路径」的自检兜底（Q106）。

### C2. 上游 pin 的机器可读记录（`.upstream-pin`）

**上游**：无（goose/octop 都不管「谁在抄我们」）。

**quill**：`.upstream-pin` 记 `vX.Y.Z <sha1>`，由 `.upstream-check.mjs` 与
`UPSTREAM.md`、`vendor/goose/Cargo.toml` **三方比对**。

**代价**：本地**无法**验证那份拷贝真的就是那个 commit（`vendor/goose` 取回后是 git
检出、能 diff，但 `.upstream-pin` 记的是「我们打算用哪一版」，不是文件级证明）——
`UPSTREAM.md` 里那句限制仍然成立。

---

## 复现本文档所依据的事实

```bash
# A1：内核循环在哪、头注怎么写的
sed -n '1,20p' crates/quill-core/src/turn.rs

# A2：阶梯在核心里、错误分类在壳里的映射
grep -n "REMOVAL_PERCENTAGES" crates/quill-core/src/compaction.rs
grep -n "provider_error_is_not_guessed" -A3 crates/quill-server/src/chat_compaction.rs

# A3：估算器与它的口径
grep -n "impl TokenEstimator" -A12 crates/quill-server/src/chat_compaction.rs

# A4：落库在壳
grep -n "fn finish_turn" crates/quill-server/src/api_chat.rs

# A5：零读写的列
grep -c "checkpoint_key\|error_state\|replan_count" crates/quill-server/src/*.rs

# A6：记忆在内核、两条传输（stdio 入口在 CLI、进程内在 quill-core::builtin）
grep -n "pub fn remember\|pub fn retrieve" crates/quill-core/src/memory.rs
grep -n "run_mcp" crates/quill-cli/src/main.rs
grep -n "pub fn serve_on_duplex" crates/quill-core/src/memory.rs
grep -n "pub fn spawn" crates/quill-core/src/builtin.rs

# A7：回滚是原地删除（goose 那条），fork 没做（octop 那条）
grep -n "rollback_from_message" crates/quill-server/src/chat_repo.rs
grep -n "sessions/{id}/rollback" crates/quill-server/src/routes.rs
grep -n "fork" .octop-ref/octop/src/octop/api/routers/chat/history.py

# B1：派工记账会话（建它的是 teams_repo，不是 api_teams —— 这条命令原来是错的，已改）
grep -c "team_leader" crates/quill-server/src/api_chat.rs crates/quill-server/src/teams_repo.rs

# C1/C2：两道自造门禁
node .layer-guard.mjs --self-test
node .upstream-check.mjs
```
