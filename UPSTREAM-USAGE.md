# UPSTREAM-USAGE —— 我们用到上游的每一个机制，逐条给出处

这份文件回答一件事：**我们说「跟上游对齐」的时候，凭什么。**

`WORKING.md` 的规矩是：引用上游必须带 `file:line`，并区分三种状态 ——
**已接入**（真用了它的形状，代码里有对应实现）、**只读对齐**（结构一致，代码我们自己写）、
**我们的选择**（上游没有对应做法，必须写明是自创）。但这些引用原先散在代码注释和
`BACKLOG.md` 里，没有索引，所以没人能把它们**当一集合起来核**，也已经有几处悄悄烂掉了
（裸写「文件名 + 行号」那次，见 `BACKLOG.md` 的 B6-3）。

`node .provenance-check.mjs` 是这份文件的对照面：它把下面每一个 `file:line` 都真的打开核一遍。

**反过来那一半在 [`docs/UPSTREAM-DIVERGENCES.md`](docs/UPSTREAM-DIVERGENCES.md)**（queue Q079）：
哪些地方我们**知道上游怎么做却故意不那样做**，每条写清上游做法、「quill 怎么做」与**代价**。
两份合起来才是完整的「跟上游对齐」交代 —— 只看这一份会把「我们没跟的部分」当成不存在。

---

## 怎么读这张表

**列顺序固定为：`机制 | 我们的落点 | 上游出处 | 状态 | 差别`。**
（前两节各有一张同列序的表；「刻意没抄」与「核不到的地方」两张表列序见各自表头。）

- **我们的落点** —— 本仓库里的 `file:line`。可以不写行号（写纯路径），那表示「文件在，但
  这一处没钉行号」。
- **上游出处** —— `vendor/goose/…`（内核）、`.octop-ref/octop/…` 或 octop 相对路径（壳）、
  `vendor/openoctopus-frontend/…`（第三个项目，见 `UPSTREAM.md` 的警告）。**octop 相对路径
  必须在 sparse 集合内**，否则归入「核不到的地方」。
- **状态** —— 只能是 `已接入` / `只读对齐` / `我们的选择` 三选一。
- **差别** —— 一句话说清哪里不一样。不许写「无差别」就完事。

**引用必须写全路径。** 只写 octop 的文件名再加行号会被 `.provenance-check.mjs` 判为
格式错误（退出码 1），因为 octop 有两个同名 skillhub_market.py，写裸名读者会落到另一份上。

> **本文件里的约定：反引号 = 这是一条引用。** 散文片段（命令、路径模板如
> `skills/{slug}/SKILL.md` 一类）不要加反引号 —— 门禁会把它们当引用去核，然后报一堆
> 查不出所以然的失败。

---

## 机制索引 · goose（内核）

| 机制 | 我们的落点 | 上游出处 | 状态 | 差别 |
|---|---|---|---|---|
| token 口径：入参**已含**缓存，缓存两项是子集 | `crates/quill-provider/src/types.rs:148-157` | `vendor/goose/crates/goose-provider-types/src/conversation/token_usage.rs:88-91` | 已接入 | 语义逐字照抄；`Option<i32>` 换成 `Option<u32>`，NULL=没报、0=真是 0 的区分不变 |
| Usage 的五个字段形状（含 `total_tokens`） | `crates/quill-store/migrations/0008_token_metrics.sql:4-12` | `vendor/goose/crates/goose-provider-types/src/conversation/token_usage.rs:93-101` | 已接入 | 我们只落 cache_read/cache_write 两列；`total` 实时由 input+output 算，不入库（多一份存储就多一处能漂移的真相来源） |
| 缓存两列必须可空 | `crates/quill-store/migrations/0008_token_metrics.sql:16-24` | `vendor/goose/crates/goose-provider-types/src/conversation/token_usage.rs:96-100` | 已接入 | goose 的 `Option` 天然可空；SQLite 用「无默认值的可空列」等价实现，重建表时逐列照抄 0001 |
| 逐字段求和、跳过 `None` 而不是折成 0 | `crates/quill-server/src/api_chat.rs:850-853` | `vendor/goose/crates/goose-provider-types/src/conversation/token_usage.rs:103-113` | 已接入 | 与 `sum_optionals` 的四个分支同构；全 `None` 时仍然是 `None` |
| 总量按减法：未缓存入参 = 入参 − 缓存读 − 缓存写 | `crates/quill-provider/src/types.rs:185-201` | `vendor/goose/crates/goose-provider-types/src/canonical/model.rs:79-80` | 已接入 | 我们**不显式算 uncached**，直接拿已含缓存的入参当分母（等价，少一处可写错的地方） |
| 多轮求和后把 cache_read 夹到不超过 input | `crates/quill-server/src/api_chat.rs:833-838` | `vendor/goose/crates/goose-provider-types/src/canonical/model.rs:79-80` | 我们的选择 | goose 那边没有跨轮累加这层。夹取只为兜「某轮报 input=None、另一轮报 cache_read」的半真值组合；单轮（`rounds<=1`）不夹，原始口径原样存 |
| 自定义 agent = frontmatter + 正文即 instructions | `crates/quill-store/migrations/0004_expert_persona.sql:2-5` | `vendor/goose/documentation/docs/guides/context-engineering/custom-agents.md:13` | 已接入 | goose 是磁盘上的 `.md` 文件；我们是 SQLite 的列，映射写在同一段注释里 |
| frontmatter 的 name / description / model 三个字段 | `crates/quill-store/migrations/0004_expert_persona.sql:9-13` | `vendor/goose/documentation/docs/guides/context-engineering/custom-agents.md:35-37` | 已接入 | 字段名与「model 可空 = 跟随默认模型」的语义照抄 |
| frontmatter 只有 name 必填 | `crates/quill-store/migrations/0004_expert_persona.sql:5` | `vendor/goose/documentation/docs/guides/context-engineering/custom-agents.md:51` | 已接入 | 我们把「不写 model」直接表达成 SQL 的 NULL，不引入哨兵值 |
| 子 agent 独立 config + 独立 session | `crates/quill-server/src/api_dispatch.rs:133-136` | `vendor/goose/crates/goose/src/agents/subagent_handler.rs:46` | 我们的选择 | **未接入**：只对齐了接口形状，派工仍只记账不执行（响应里 `executed:false`）。执行器是 M2 的活，见 `BACKLOG.md` 的 B2 |
| 流式是**唯一的**模型调用形态，没有「一次性」那一支 | `crates/quill-server/src/api_chat.rs`（`ReplyMode::Streamed` / `streamed_round`） | `vendor/goose/crates/goose/src/agents/reply_parts.rs:342` | 已接入 | goose 只有 `stream_response_from_provider` 一个入口，返回**流**而不是完整响应。我们**保留**了老的一次性路由（`ReplyMode::Once`）且语义一个字没改 —— 那是 octop 的契约路由，不能为了对齐上游改语义 |
| 流式请求在可观测字段上打标记（`gen_ai.request.stream = true`） | `crates/quill-server/src/api_chat.rs`（`streamed_round` 的 `eprintln!("[chat] 流式一个字都没吐就失败…")`） | `vendor/goose/crates/goose/src/agents/reply_parts.rs:328` | 我们的选择 | 我们**没有** OTel spans，只有 `[chat]` 行日志。这是与上游真实存在的可观测性差距，不是等价实现；接 traces 记在 B3 |
| 记忆（memory）：一个分类一个 `.txt`，`# 标签` 行 + 正文 + 空行分条 | `crates/quill-core/src/memory.rs`（`remember` / `retrieve`） | `vendor/goose/crates/goose-mcp/src/memory/mod.rs:260-325` | 已接入 | 语义逐字照抄（含「空行分条、首行 `#` 才是标签、否则归 `untagged`」）。**本地目录 `.goose/memory` → `.quill/memory`**（quill 是自己的产品，用 `.goose` 是错的） |
| 记忆的四个工具（`remember_memory` / `retrieve_memories` / `remove_memory_category` / `remove_specific_memory`） | `crates/quill-core/src/memory.rs`（`#[tool]` 方法） | `vendor/goose/crates/goose-mcp/src/memory/mod.rs:390-507` | 已接入 | 工具**名字**与**描述**照抄（名字是模型看到的东西，不能漂）；服务器自报名 `goose-memory` → `quill-memory` |
| 记忆分类名必须是单个文件名分量（空 / `*` / `.` / `..` / 含 `/\:` / Windows 保留名一律拒） | `crates/quill-core/src/memory.rs`（`get_memory_file`） | `vendor/goose/crates/goose-mcp/src/memory/mod.rs:184-215` | 已接入 | 逐条照抄（含 `CON`/`PRN`/`AUX`/`NUL`/`CLOCK$`/`COM1..9`/`LPT1..9` 与上标 ¹²³）。这是**安全边界**：分类名来自模型 |
| 全局记忆在启动时拼进服务器 instructions（`Global Memories:` 段） | `crates/quill-core/src/memory.rs`（`instructions_with_global_memories`） | `vendor/goose/crates/goose-mcp/src/memory/mod.rs:143-170` | 已接入 | 行为照抄；全局目录用 `default_global_memory_dir()` 现算（`%APPDATA%` / `$XDG_CONFIG_HOME` / `$HOME/.config`），**没引 goose 用的 `etcetera`** |
| 内置扩展的 stdio 入口（`goose mcp memory`） | `crates/quill-cli/src/main.rs`（`run_mcp` → `quill mcp memory`） | `vendor/goose/crates/goose-mcp/src/mcp_server_runner.rs:36-49` | 已接入 | 调用序列照抄（`ServiceExt::serve(stdio())` → `waiting()`）。goose 另有**进程内**那条（`vendor/goose/crates/goose/src/agents/extension_manager/builtin.rs:19-33` 的 `tokio::io::duplex`）；quill 只走 stdio，因为它的 `mcp_client` 只铺了 stdio |

## goose 刻意没抄

| 上游做法（出处） | 我们怎么做的 | 为什么这么定 |
|---|---|---|
| `vendor/goose/crates/goose/src/agents/reply_parts.rs:342` | goose 那条是**进程内**的 `MessageStream`（`vendor/goose/crates/goose/src/agents/reply_parts.rs:21` 从 `providers::base` 引入），内核自己消费，没有对外的 HTTP 端点 | quill 的前端是浏览器，只能通过 HTTP 拿流。我们新开的 `POST /api/sessions/{id}/messages/stream`（`crates/quill-server/src/api_chat_stream.rs`）**整份事件协议是我们自己定的** —— 不是抄来的。也正因为是自己加的，它登记在 `EXTRA_ROUTES` 而不是 `CONTRACT_ROUTES` |
| 同上 | 我们发一帧 `discard`，点名**要抹掉的原文**（不是长度） | 工具往返那一轮的正文会被下一轮覆盖掉（老行为里直接被 `reply` 覆盖、不落库）。浏览器没有终端那种「把这一段重画一遍」的能力，只能显式告诉前端抹哪一段。给长度不行的：中文与代理对会让「删 N 个字符」算错 |
| 同上 | 上游不支持 `stream: true` 时**一个字都没吐出来**就退回一次性调用 | 不少 OpenAI 兼容端点对流式的支持不完整（老版本 llama.cpp、部分网关直接回 400）。流式是锦上添花，不能让它把「聊天」变成不可用。已经吐过字的**不**退回 —— 补一次会让同一段话显示两遍，那比报错更糟 |
| `vendor/goose/crates/goose-provider-types/src/conversation/token_usage.rs:88-91` 里「provider 单独报 cache 时要把它折进 input_tokens」这条假设 | 两条口径都收（折进去的与分开报的都能解析），但事后**夹取**到 `input` 之内 | 这条假设对 Anthropic 与官方文档相反，而 goose 源码里判不了对错。我们不替上游改口径，也不把一个可能错的假设当契约 —— 记在 `BACKLOG.md` 的 B3-2 |

---

## 机制索引 · Octop（产品壳）

| 机制 | 我们的落点 | 上游出处 | 状态 | 差别 |
|---|---|---|---|---|
| 专家库列表排除 `default` | `ui/web/src/experts/library.ts:9-11` | `.octop-ref/octop/src/octop/infra/agents/experts/catalog.py:852-853` | 已接入 | 逐字同构；`.library-check.mjs:61-79` 把它当门禁钉住 |
| 专家库排序：general-assistant 置顶，其余按 id 升序 | `ui/web/src/experts/library.ts:122-123` | `.octop-ref/octop/src/octop/infra/agents/experts/catalog.py:857` | 已接入 | 排序键逐字照抄 |
| SKILL 即工具（`tool_name` 是「技能进不进模型」的唯一开关） | `crates/quill-server/src/skills_repo.rs:3-6` | `.octop-ref/octop/dashboard/src/api/types/skill.ts:3-5` | 已接入 | 字段与约定照抄；quill 的 `tool_name` 恒等于 slug（有测试钉住） |
| 团队卡片最多露出 6 个成员头像，多出来并成 +N | `ui/web/src/experts/TeamsTab.tsx:29-30` | `.octop-ref/octop/dashboard/src/pages/Experts/components/TeamCard.tsx:56` | 只读对齐 | 数字照抄；我们是手写组件，不引 antd |
| 解析成员名册时跳过查不到的 id | `ui/web/src/experts/TeamsTab.tsx:308-315` | `.octop-ref/octop/dashboard/src/pages/Experts/components/TeamCard.tsx:80-82` | 只读对齐 | 它 `continue` 之后就没下文了；我们把跳过的条数单列成「缺失」一行。**跳过不等于没这回事** |
| 成员名册按整组替换，不做增量追加 | `ui/web/src/experts/TeamsTab.tsx:553` | `.octop-ref/octop/dashboard/src/pages/Experts/components/TeamMemberPicker.tsx:35-51` | 已接入 | 契约（PATCH 的 `member_ids` 是全量）与它一致；`crates/quill-server/tests/team_http.rs:524-528` 钉住 |
| 团队徽标用 lucide `Users` | `ui/web/src/experts/icons.tsx:126` | `.octop-ref/octop/dashboard/src/pages/Experts/components/TeamCard.tsx:231` | 只读对齐 | 同一个 `strokeWidth` 观感，但手写描边 SVG，不引 `lucide-react` |
| 视图记忆沿用同一个 localStorage key | `ui/web/src/experts/ExpertsPage.tsx:45-46` | `.octop-ref/octop/dashboard/src/pages/Experts/index.tsx:70` | 已接入 | **逐字复用同一个字面量** `octop:experts-view`；`ui/web/src/experts/ExpertsPage.test.tsx:130` 断言它 |
| 卡片/表格视图切换 | `ui/web/src/experts/ExpertsPage.tsx:52` | `.octop-ref/octop/dashboard/src/hooks/useCardTableView.ts:11` | 只读对齐 | 它是 `useState` + `showCardView`；我们去掉用不上的 `isMobile` 那一支 |
| 专家表格的列与文案 | `ui/web/src/experts/ExpertTable.tsx:11` | `.octop-ref/octop/dashboard/src/pages/Experts/components/AgentExpertsTable.tsx:293-428` | 只读对齐 | 列名逐字；数据源按 quill 实情映射：「状态」是 `default_enabled`（没有常驻 agent 进程，所以没有 running/stopping，也不画启停按钮） |
| 「设为默认」用一个小号开关 | `ui/web/src/experts/experts.css:350` | `.octop-ref/octop/dashboard/src/pages/Experts/components/AgentCard.tsx:364-372` | 只读对齐 | 位置与尺寸对齐，控件手写（不引 antd `Switch`） |
| 删除前的二次确认 | `ui/web/src/experts/ExpertsUi.tsx:3` | `.octop-ref/octop/dashboard/src/pages/Experts/components/AgentCard.tsx:452-472` | 只读对齐 | 同样是 Popconfirm 语义，手写等价件 |
| 专家 ID 可点击复制 | `ui/web/src/experts/experts.css:301` | `.octop-ref/octop/dashboard/src/pages/Experts/index.module.less:2098` | 只读对齐 | 类名沿用它，便于对账 |
| 会话指标的十个字段名与展示顺序 | `ui/web/src/chat/sessionMetrics.ts:39-51` | `.octop-ref/octop/dashboard/src/pages/Chat/utils/trajectoryModel.ts:535-546` | 已接入 | 与 `.octop-ref/octop/dashboard/src/api/modules/trajectory.ts:34-45` 的接口逐字段同名同序，便于对照 |
| 没量到的指标（null）整项不出现 | `ui/web/src/chat/sessionMetrics.ts:65-74` | `.octop-ref/octop/dashboard/src/pages/Chat/utils/trajectoryModel.ts:565-570` | 已接入 | 与它一致：`null` 不是 0。我们多一条硬规矩 —— quill 不记工具耗时与首字延迟，那两格**永远**不显示 |
| 时长的显示格式 | `ui/web/src/chat/sessionMetrics.ts:110-120` | `.octop-ref/octop/dashboard/src/pages/Chat/utils/trajectoryModel.ts:553-563` | 已接入 | 逐字抄（`—` / `ms` / `s` / `m..s` 四档） |
| 上下文环的分段键与顺序 | `crates/quill-server/src/session_metrics.rs:210-224` | `.octop-ref/octop/dashboard/src/pages/Chat/components/ContextWindowRing.tsx:16-24` | 已接入 | 五个分段键与顺序逐字；配色自己定（它的 `SEGMENT_COLORS` 是十六进制色板，我们走 CSS 变量） |
| 上下文占用：真的占了非零就至少显示 1% | `crates/quill-server/src/session_metrics.rs:229-243` | `.octop-ref/octop/dashboard/src/pages/Chat/components/ContextWindowRing.tsx:144-146` | 已接入 | 规则照抄，包括「真的没用就是 0%，不抬成 1%」 |
| 环的总占用来自实测入参，不做字符数估算 | `crates/quill-server/src/api_chat.rs:447-454` | `.octop-ref/octop/dashboard/src/pages/Chat/components/ContextWindowRing.tsx:144-146` | 已接入 | 它自己的注释也是「provider input usage owns the total bar width」；我们的 `used_tokens` 取最后一条 assistant 消息的 `input_tokens` |
| 构成段是估算值，前面加 `~` | `ui/web/src/chat/ContextRing.tsx:112-117` | `.octop-ref/octop/dashboard/src/pages/Chat/components/ContextWindowRing.tsx:254` | 已接入 | 做法照抄：靠 `~` 前缀声明这是估算，而不是用一整句话解释符号。分段数值本身仍是服务端量的字符数，quill 没有分词器 |
| 上下文图的数据源接口 | `crates/quill-server/src/api_chat.rs:447` | `.octop-ref/octop/dashboard/src/pages/Chat/components/ContextWindowRing.tsx:144-146` | 只读对齐 | 形状对齐（used / max / 分段），路由与字段名是我们自己的 |
| 会话级统计的产出规则（算不出来就是 null） | `crates/quill-server/src/session_metrics.rs:1-25` | `.octop-ref/octop/dashboard/src/api/modules/trajectory.ts:34-45` | 只读对齐 | 它十个指标都有数据源；quill 只能诚实产出一部分，缺的那些宁可不出现在界面上 |
| 登录 / 首管引导的端点清单 | `crates/quill-server/src/api_auth.rs:3-12` | `.octop-ref/octop/dashboard/src/api/modules/auth.ts:11-17` | 已接入 | 路由与响应形状照抄；我们不抄 `captcha_token` 与 OIDC/OAuth/LDAP/改密码（抄过来就是恒定失败的字段），`logout` 我们返 200 + `revoked` 而不是 204 |
| 令牌只有一种，且必须由服务端按账号状态裁决 | `crates/quill-server/src/auth.rs`（`CompositeTokenResolver::confirm_identity`） | `.octop-ref/octop/dashboard/src/api/request.ts:251-254`、`.octop-ref/octop/dashboard/src/api/request.ts:333-341` | 已接入 | 它每个请求统一带 `Authorization: Bearer`，401 一律清令牌跳登录，服务端是唯一裁判。**我们多一种令牌**（`QUILL_TOKENS` 环境变量直发令牌，octop 没有这个机制），但对齐的正是它最要紧的那条性质：令牌解析必须回库核状态与角色，不能凭配置放行。差别见下条 |
| 明确拒绝「把鉴权关掉」 | `crates/quill-server/src/auth.rs`（库句柄未就绪时一律按令牌无效处理，不放行） | `.octop-ref/octop/dashboard/src/api/modules/auth.ts:306-308` | 只读对齐 | 它的 `disableAuth` 直接 `Promise.reject`，注释写着「Octop cannot disable auth」。我们没有这个开关，也没有对应的后门：查不到账号行就拒 |
| MCP transport 枚举 | `crates/quill-store/migrations/0007_mcp_transport_alignment.sql:26-28` | `.octop-ref/octop/dashboard/src/api/modules/connectors.ts:128` | 我们的选择 | 它只有 `stdio` 与 `streamable_http`；我们多两个 —— `sse`（MCP 规范的另一种 HTTP 传输）与 `builtin`（quill 内建的伪服务器，不来自外部配置）。多出来的两个不是抄来的 |
| 技能卡片的网格下限 | `ui/web/src/skills/hub.css:142-173` | `.octop-ref/octop/dashboard/src/pages/Experts/index.module.less:27` | 只读对齐 | 结构抄 320px 的 `auto-fill + minmax`，下限改成 240px，并在同一段里给出实测列数表。技能市场那页自己的 less 不在 sparse 内（见「核不到的地方」） |
| 技能卡片的列表顺序：已装的排前面 | `ui/web/src/skills/HubSkillList.tsx:248-254` | `.octop-ref/octop/dashboard/src/pages/Agent/Skills/components/SkillHubTab.tsx` | 只读对齐 | 只动位置不动内容，列表条数与上游一致；但出处那页离线核不到 |
| 技能卡片显示 slug 与版本 | `ui/web/src/skills/HubSkillList.tsx:205-212` | `.octop-ref/octop/dashboard/src/pages/Agent/Skills/components/SkillHubTab.tsx` | 我们的选择 | 它的卡片只显示 `name`，而实测有一批技能的 `name` 是占位串（`martin-pdf` 的 name 字面就是 `pdf`），去掉 slug 之后三张卡片全都叫「pdf」 |
| 卡片无图标时的占位图标 | `ui/web/src/experts/icons.tsx:138-139` | `.octop-ref/octop/dashboard/src/pages/Agent/Skills/components/SkillHubTab.tsx` | 只读对齐 | 同款描边手写，不引 `lucide-react`；出处那页离线核不到 |
| 卡片左下角的下载数 | `ui/web/src/experts/icons.tsx:148-149` | `.octop-ref/octop/dashboard/src/pages/Agent/Skills/components/SkillHubTab.tsx` | 只读对齐 | 同上 |
| 技能落盘路径与上游同构 | `crates/quill-server/src/api_extensions.rs:1483` | `.octop-ref/octop/dashboard/src/api/types/skill.ts:3-5` | 只读对齐 | 我们落成 `<slug>.md` 单文件，它落成 `skills/{slug}/SKILL.md` 目录；语义同构，布局不同 |
| SkillHub 客户端的安全上限 | `crates/quill-server/src/skillhub/http.rs:18-22` | `.octop-ref/octop/src/octop/infra/agents/experts/skillhub_market.py:34-40` | 已接入 | 四个数照抄它，**但常量的定义在 sparse 之外**（`src/octop/infra/skills/skillhub_common.py`），手边只有 import 处；数字本身离线无法复核 |
| 上游地址可配置 | `crates/quill-server/src/skillhub/http.rs:10` | `.octop-ref/octop/src/octop/infra/agents/experts/skillhub_market.py:35` | 已接入 | 同样是「用同一个常量、但定义在 sparse 外」 |
| 拉取超时与每页条数 | `crates/quill-server/src/skillhub/http.rs:18` | `.octop-ref/octop/src/octop/infra/agents/experts/skillhub_market.py:51-52` | 已接入 | 30 秒与 100 条，与它的 `_HTTP_TIMEOUT` / `_SKILLSET_PAGE_SIZE` 同值 |
| 解 zip 的攻击面检查 | `crates/quill-server/src/skillhub/unpack/mod.rs:1-8` | `.octop-ref/octop/src/octop/infra/agents/experts/skillhub_market.py:37-40` | 已接入 | 用 Rust 的 `zip` crate 解，不用自己写解压器；四个上限同值 |
| 专家市场的上游就是 SkillHub 的 skillsets | `crates/quill-server/src/api_expert_market.rs:57-110` | `.octop-ref/octop/dashboard/src/api/modules/expertMarket.ts:99-112` | 已接入 | 它的 `/experts/hub` 与它的 `/skill-packages/hub` 指向同一份 `/api/v1/skillsets`；**没有第二个上游客户端**，直接复用已有的 `skillhub` / `skillhub_unpack` |
| 装市场专家时人格取 skillsets/<slug>.md → skillsets/ 下第一篇 → identify.md | `crates/quill-server/src/skillhub/unpack/mod.rs:309-353` | `.octop-ref/octop/src/octop/infra/agents/experts/skillhub_market.py:611-621` | 已接入 | 顺序照抄。**不能直接用 `unpack`**：它按 `sanitize_name` 把条目拍平成 basename，于是编排提示与它引用的技能正文拍平后撞名，分不出哪篇才是人格 |
| 装包前先取一次技能集详情 | `crates/quill-server/src/skillhub/endpoints.rs:53` | `.octop-ref/octop/src/octop/infra/agents/experts/skillhub_market.py:389` | 已接入 | 它的 manifest 缺失兜底本来就靠这个端点；我们额外用它兜「包里没有可用 manifest」的情形 |
| 装进来的技能一律 `enabled=false` | `crates/quill-server/src/api_extensions.rs:1073` | `.octop-ref/octop/src/octop/infra/agents/experts/skillhub_market.py:388-418` | 已接入 | 与已有的技能包安装同一套决策（实测 13 个技能就能把 8192 上下文顶爆） |
| 市场专家 id = `hub-<slug>` | `crates/quill-server/src/api_expert_market.rs:41-54` | `.octop-ref/octop/src/octop/infra/agents/experts/skillhub_market.py:50` | 我们的选择 | 它用 `skillhub-skillset-` 前缀；我们缩短成 `hub-` 且**超长直接报错不截断** —— 截断会造出撞名专家（见 `BACKLOG.md` 的 B1-6） |
| 会话 hover 才出现操作按钮 | `ui/web/src/chat/chatShell.css:169` | `.octop-ref/octop/dashboard/src/pages/Chat/chatSidebar.partial.less:248` | 只读对齐 | `opacity` 行为对齐，按钮位置因侧栏宽度不同而不同 |
| 两栏布局的基线宽度 238px | `ui/web/src/chat/chatShell.css:15` | `vendor/openoctopus-frontend/src/index.css:70` | 已接入 | 注意这是**第三个项目**（OpenOctopus，CSS 移植基准），不是 Octop。见 `UPSTREAM.md` 末尾的警告 |
| 表格视图下点编辑 | `ui/web/src/experts/ExpertsPage.tsx:404` | `.octop-ref/octop/dashboard/src/pages/Experts/index.tsx:870` | 我们的选择 | 它开一个 Drawer；我们沿用现有内联表单挂在表格下方，少一层模态 |
| 对话页的上下文图只在有实测值时出现 | `ui/web/src/chat/ChatPage.tsx:541-544` | `.octop-ref/octop/dashboard/src/pages/Chat/components/ContextWindowRing.tsx:144-146` | 我们的选择 | 同一条纪律（没量过就不画），但挂在输入框下方而不是消息流里 |
| 加载中的图 | `ui/web/src/usage/ContextWindowChart.tsx:85-87` | `.octop-ref/octop/dashboard/src/pages/Chat/components/ContextWindowRing.tsx:144-146` | 我们的选择 | 我们把「还没量过」「拉失败」「有数据」三态分开渲染并把错误原样说出来；它加载中直接不画，读屏用户听到的是「这块不存在」 |
| 图表库 | `ui/web/src/charts/echarts.ts` | `.octop-ref/octop/dashboard/package.json:48` | 我们的选择 | **它的界面根本不用 ECharts**（依赖里只有 recharts），上下文图是手写 SVG。所以我们的「内容指纹 + `updateKey`」是**自创机制，不是参考实现**，见下一节 |

## Octop 刻意没抄

| 上游缺陷（出处） | 我们怎么做的 | 为什么这么定 |
|---|---|---|
| 装包时直接取 zip 列表里的第一篇正文，**不排序** | `crates/quill-server/src/api_extensions.rs:1110` 先算出安装计划（`InstallPlan`），再逐条写入 | `.octop-ref/octop/src/octop/infra/agents/experts/skillhub_market.py:610-616` 里 `selected = preferred if preferred in zip_names else skillset_files[0]`。第一篇取决于 zip 的物理顺序，重打包就换一个结果 |
| 扁平的 `skillSlugs` 原样透传，**没有去重** | `crates/quill-server/src/api_extensions.rs:1076` 保证「一个技能名最多出现一次」，撞名的逐条点名 | `.octop-ref/octop/src/octop/infra/agents/experts/skillhub_market.py:652-655` 直接 `[str(s).strip() for s in raw]` 返回。同名会被当成两条装两次，而磁盘上只有一份正文 |
| 上下文图是**手写 SVG**，仓库里没有 ECharts | `ui/web/src/charts/EChart.tsx:84-94` 用「option 内容指纹」判断该不该重画，`ui/web/src/usage/ContextWindowChart.tsx:205` 用 `updateKey` 补闭包里的翻译 | `.octop-ref/octop/dashboard/src/pages/Chat/components/ContextWindowRing.tsx:273-282` 是手写的 `<svg>` + `<circle>`；`.octop-ref/octop/dashboard/package.json:48` 的绘图依赖是 recharts。**我们这套机制是自创的**，不能说成参考实现 |
| 技能卡片只显示 `name`，丢掉 slug 与版本 | `ui/web/src/skills/HubSkillList.tsx:205-212` 把 slug 与 version 都印出来 | 见上面「技能卡片显示 slug 与版本」那一行：实测 `name` 是占位串时会撞名 |
| 成员名册解析到未知 id 就 `continue` | `ui/web/src/experts/TeamsTab.tsx:308-315` 跳过但把缺失条数单列 | `.octop-ref/octop/dashboard/src/pages/Experts/components/TeamCard.tsx:80-82`。静默丢弃会让「团队有 5 个人」变成界面上只显示 3 个徽标 |
| 两个永远走不到的错误码 | `crates/quill-server/src/api_teams.rs:12-16` 明写「等真出现这两件事时再随功能一起加」 | quill 没有常驻 agent 进程、没有团队分享。写一个永远走不到的分支等于凭空造状态 |
| 包内 manifest 当**必备**，缺了整单 `PACKAGE_INVALID` | `crates/quill-server/src/skillhub/unpack/mod.rs:350` 允许它缺失，退回上游详情接口给的 `skillSlugs`，两者都没有就装「只有人格」的专家并如实回报 | `.octop-ref/octop/src/octop/infra/agents/experts/skillhub_market.py:608`。人格本身完整可用，为一个描述性字段把整单废掉比装上一个用户能看见也能改的专家更糟 |

## Octop 核不到的地方（sparse 集合之外）

`.octop-ref/octop` 是 sparse checkout（集合见 `UPSTREAM.md`）。下面这些引用**在离线环境里
无法逐行核对** —— 这是信息，不是失败：写在这里意味着「我知道自己缺哪一半证据」，
而不是「我编了一个行号」。`.provenance-check.mjs` 把它们归为 `offline-unverifiable`：
**可见，但不因此让门禁失败**（本地检出就这副样子，不是引用腐烂）。

| 引用 | 为什么核不到 |
|---|---|
| `src/octop/infra/skills/skillhub_common.py` | 不在 sparse 集合内。四个安全上限与 `DEFAULT_SKILLHUB_HOST` 的**定义**在这里，我们只核得到 `src/octop/infra/agents/experts/skillhub_market.py:34-40` 的 import 处 |
| `src/octop/infra/skills/skillhub_market.py` | 不在 sparse 集合内。**octop 有两个同名文件**，端点常量（SEARCH/DOWNLOAD/RANKING）在这一份里；专家那份（手边 1354 行内容）里没有这些常量 |
| `dashboard/src/pages/Agent/Skills/components/SkillHubTab.tsx` | 不在 sparse 集合内。技能卡片的形态、`displaySkills`、`hubCardIconFallback`、`hubCardStat` 都在这里 —— 上面四条相关引用都因此核不到 |
| `dashboard/src/pages/Agent/Skills/index.module.less` | 不在 sparse 集合内。技能市场页自己的网格样式；`ui/web/src/skills/hub.css:142-173` 抄的 320px 下限，我们在 sparse 内能找到同一个字面量（`dashboard/src/pages/Experts/index.module.less:27`），但那不是它被抄的那一页 |
| `dashboard/src/pages/Control/**` | 不在 sparse 集合内。涉及这一处的任何对比都缺一半证据 |
| `crates/quill-server/src/skillhub/endpoints.rs:95` 声称照抄的榜单类型集合 | 定义处不在 sparse 集合内，稀疏集里搜不到这个符号 —— **无法核实**，见交付报告 |
| octop 的**服务端**鉴权实现（JWT 签发/校验、账号停用如何影响已签发的令牌） | src/octop/ 在 sparse 内只有 config.py、launch.py、__main__.py 与 infra/agents/experts/；没有任何 auth/user 模块。所以「停用某个账号后，他手里已签发的 JWT 还能不能用」这条**核不到出处**。这一轮据以下面三条前端事实定的方案：令牌只有一种（`.octop-ref/octop/dashboard/src/api/request.ts:251-254`）、401 一律清令牌（`.octop-ref/octop/dashboard/src/api/request.ts:333-341`）、不许关鉴权（`.octop-ref/octop/dashboard/src/api/modules/auth.ts:306-308`） |

---

## 维护这份文件

```bash
node .provenance-check.mjs            # 逐条核 file:line；退出码非 0 = 有引用真的烂了
node .scripts/provenance-selftest.mjs # 给判定逻辑本身的合成输入自测（不碰真实文件）
```

改完上游基线（`UPSTREAM.md`）或增删一行机制，**两条都要跑**：一条核出处，
一条证明核出处的那段逻辑没写坏。
