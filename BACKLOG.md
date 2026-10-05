# BACKLOG

已确认要做但还没做的项。每条写清**为什么现在不做**、**做的时候卡在哪**，避免下次重新调研。
需求来源是 `REQUIREMENTS.md`；Octop 的对应实现在 `.octop-ref/octop`（用 `node .octop-ref/graph.mjs <cmd>` 查，不要整读文件）。

---

## B 层 · 专家图标选择器 + 每个专家自己的欢迎语

**状态**：已确认要做，用户明确说「不是什么大事」，排在 D 层之后。

**Octop 对应物**：
- `dashboard/src/pages/Experts/components/iconForName.tsx` —— `iconMap` 30 个图标
  （`sparkles globe user users rocket fingerprint video palette home baby cpu server wrench heart
  mail zap terminal presentation activity network utensils bell coffee cloudy cloud laptop monitor
  wifi satellite name`），`FILE_META_MAP` 按人格文件名上色
  （`SOUL.md` 紫 Sparkles / `IDENTITY.md` 粉 Fingerprint / `USER.md` 蓝 User / `AGENTS.md` 绿 BookOpen /
  `TOOLS.md` 橙 Wrench / `BOOTSTRAP.md` 深橙 Rocket / `HEARTBEAT.md` 红 Heart / `SKILL.md` 紫 Zap /
  `MEMORY.md` 青 Layers / `PROACTIVE.md` 橙 Bell）。
- `components/ExpertAvatarPicker.tsx`（4.2 kB）—— 上传/移除头像。
- `components/WelcomeConfig.tsx`（13.6 kB）—— 欢迎语 + 快捷提问卡片。

**卡点（需要先决策）**：Octop 的图标全来自 `lucide-react`，quill 没有这个依赖。两个选项：
1. 加 `lucide-react` 依赖（引依赖，与 quill「依赖尽量少且 vendor」的风格有张力）；
2. 把这 30 个图标 SVG vendor 进 `ui/web/src/experts/icons.tsx`（lucide 是 ISC 许可，可抄，
   零依赖，但以后加图标要手工同步）。

**要加的迁移**：`0006_expert_appearance.sql`，字段 `icon_name` / `icon_url` / `color`，
再加 per-expert 的欢迎语与快捷提问（现在是静态库 `library.ts` 里的，派生专家靠
`source_template` 借用模板的）。

**为什么现在不做**：要先定上面那个依赖策略，而它影响仓库的依赖政策，不该顺手决定。

---

## C 层 · Octop 有、quill 无对应后端能力的功能

**明确不抄**，因为抄了就是「点了没反应」的假按钮。记在这里是为了下次别再纠结一遍。

| Octop 功能 | 出处 | quill 缺什么 |
|---|---|---|
| 启动 / 停止 / 重载 agent | `AgentCard.tsx:208-246,429-439` | quill 没有常驻 agent 进程（crates 里没有 `/api/agents`、没有 `start_agent`/`stop_agent`），专家只是每次请求插一条 system 消息 |
| 工作区抽屉 | `AgentCard.tsx:410-427` | 没有工作区目录概念 |
| 整理记忆 / 记忆瘦身 | `AgentCard.tsx:354` | 记忆系统未落地 |
| 技能包 / 插件 / 工具 / MCP / 记忆 / 渠道 七个 catalog 抽屉 | `AgentMoreActions.tsx:17-23` | 2026-10-06 更新：`GET /api/extensions/{mcp,skills}` 已是真实现（见下方「MCP 与 SKILL」章节），`GET /api/extensions/plugins` 仍是 501 桩，`/api/cron` 连路由都没注册（返 404）。**catalog 抽屉本身仍然没做** —— quill 只有设备页那一张 MCP 列表 |
| 发布到模板市场 / 专家市场 | `PublishExpertDrawer` / `ExpertMarketTab` | 无发布接口 |
| 共享 / 远端专家徽标 | `AgentExpertsTable.tsx:313-322` | 列表没有 `is_shared` 概念 |
| MBTI 人格标签 | `MbtiPersonaTag` | 无 MBTI 概念 |
| 2 秒轮询运行态 | `AgentExpertsTable.tsx:170-200` | 无运行态 |
| `StreamSetupGuide` 吉祥物空态 | `index.tsx:412-432` | 是 Octop 的引导流 + 美术资产 |

界面上这些能力统一用**一个**说明块列出真实接通程度，不画假开关。
数据在 `ui/web/src/capabilityGaps.ts`，对话页工具坞与专家库生成面板**共用同一份**，
避免两处各说各话。文案分两类：`partial`（存储层已通、执行层没接）与
`not-implemented`（路由压根不存在）—— 混成一句「未接通」就是在骗人。

---

## 已知不足（不是 bug，是没做）

- `PATCH /api/sessions/{id}`（会话改名）**根本没注册**，返 **405 method_not_allowed**
  （不是 501 —— 2026-10-06 复核更正；`routes.rs:93-96` 该路径只挂了 get + delete）。
  界面据此如实提示，不发请求。
- 专家级 `model` 覆盖只存不用，界面标「已保存，暂未参与路由」。
- 表格视图没有排序与分页（Octop 用 antd Table 的分页 + fixed 列），专家量级下先不做。
- 表格里的编辑是内联表单，Octop 是 Drawer；quill 没有 Drawer 基建。
- Popconfirm 浮层没用 Portal，极窄屏可能需要滚动才看得到。
  （2026-10-05 补：聊天第二侧栏的会话删除确认已改成**向下展开**并右对齐，
  见 `chatShell.css` 的 `.chat-sidebar-sessions .experts-popconfirm` 与 `.chat-titlebar .experts-popconfirm`。
  原因是外面套着 `.chat-sidebar-scroll { overflow: auto }`，默认的 `bottom: calc(100% + 6px)`
  向上展开会被裁掉。其余页面仍向上展开。）
- goose 是 **vendor 的参考源码，没有任何 crate 依赖它**（`crates/quill-testkit/fixtures/boundary/`
  里有两处提到 goose，但那是依赖边界测试的假 fixture）。所以「真调 goose 的 delegate」
  不是小改动：要么把 goose 接成依赖，要么按 `subagent_handler.rs::run_subagent_task`
  的模式在 quill 内实现子 agent 执行。

---

## D 层做完之后新发现的缺口（2026-10-05）

### 1. 团队标识有两套，dispatch 那边是假的

`/api/teams/{id}` 用的是 kebab-case 的 `team_slug`（0006 新加的列），但
`/api/teams/{id}/dispatch` **根本没读 `{id}`** —— `api_dispatch.rs:20` 写的是
`Path(_team): Path<String>`，下划线前缀，值被丢弃。实测：

```
GET /api/teams/growth-squad/dispatch?room_id=room-x&round=0   → 200 {"count":0,...}
GET /api/teams/zzz-not-a-team/dispatch?room_id=room-x&round=0 → 200 {"count":0,...}
```

即：**传一个根本不存在的团队也返回 200 和空派工记录**，读起来像「这个团队没有派工历史」，
而不是「没有这个团队」。这条路由只按 `room_id` + `round` 过滤。

**要修需要先定契约**：dispatch 到底认不认 `team_id`？认的话得改成解析 slug 并校验团队存在；
不认的话应该把 `{id}` 从路由里去掉（改成 `/api/dispatch?room_id=...`），别留一个被忽略的参数。
这是产品决定，不是纯代码修复，所以没顺手改。

### 2. 建团队会静默多出一个会话

`teams.leader_session_id` 是 `NOT NULL` 且外键指向 `sessions`（0001），所以
`POST /api/teams` 会在事务里先插一条 `team_leader` 类型的会话。它是真的会话、
不是假数据，但它会出现在对话页左侧的会话列表里（实测确实出现）。要不要在侧栏
隐藏 `kind='team_leader'` 的会话，是个产品决定；现在先如实显示。

### 3. 建团时 `workspace_path` 是拼出来的相对路径

`sessions.workspace_path` 有 `CHECK (NOT LIKE '/%')`，所以只能填相对路径。
worker 填的是 `ws/<12位hex>`，并没有真的建这个目录，也没有代码去读它。
不影响任何现有路由，但它是「填了占位符」的字段，将来真做工作区时要接上。

### 4. `TEAM_MEMBER_BUSY` / `TEAM_NOT_SHAREABLE` 故意没实现

quill 没有常驻 agent 进程（没人会「忙」），也没有团队共享功能。这两个 Octop
错误码没有触发条件，写了就是走不到的死代码。理由记在 `api_teams.rs` 文件头。

---

## 全盘审计结论（2026-10-06）

一句话：**引擎层建得挺完整，HTTP 接线层停在半路**。wiki / backup / upgrade / auth
四套能力在 crate 里已经能用，但对应路由全是 501，于是界面上有 13 处「未接通」提示、
7 个页面是骨架页。真正闭环的只有：登录（粘贴令牌）→ 聊天 → 专家/团队 CRUD → 模型管理。

### 还有 28 条 501 路由（`crates/quill-server/src/routes.rs`）

其中**界面上有真按钮、点了必失败**的（不是假按钮，但也没用）：

| 界面入口 | 打到哪 | 说明 |
|---|---|---|
| 实例/用户管理页 | `GET/POST /api/users`、`PATCH/DELETE /api/users/{id}` | `quill-control/src/service.rs` 里 login/list_users/invite 全有，只差路由 |
| 设备页 + 管理页的 MCP 卡片 | `GET/POST /api/extensions/mcp` 等 8 条 | 保存按钮真的会发请求 |
| 工作区页的 4 个按钮 | `POST /api/backup/{export,restore,verify}`、`GET /api/upgrade/check`、`POST /api/upgrade/prepare`、`GET /api/upgrade/history` | `quill-backup`(1387 行) 与 `quill-upgrade` 已是可用实现 |

### 「建了列等于没建」的实证

- `sessions.summary / summary_upto_seq / compacted_count / checkpoint_key / checkpoint_path
  / replan_count / error_state / last_error`（`0001_init.sql:245-286`）在 server **零读写**。
  也就是说 `/healthz` 和 `/api/admin/config` 都在上报 `compaction_threshold_tokens`，
  **但没有任何压缩实现**。聊天页的提示原文已经因此改成「已配置压缩阈值 N tokens，
  但压缩还没实现：超过上限不会自动摘要，需要自己新建会话」。
- `mcp_servers` / `skills` / `plugins` / `wiki_index` 四张表只出现在迁移与测试里。
- `task_dispatches.callback_at / deadline_at` 零写入。
- `teams.guidelines / max_dispatch / max_replan / max_ask_depth / state` 建了列，
  团队 CRUD 一个都不碰（`dispatch_ledger.rs` 只写台账，没有消费者）。

### 一个更正

审计报告里说「`/api/admin/instance` 后端已通、前端不接」——**不成立**。
实测 `GET /api/admin/instance` 是 **404**，那条路由压根没注册。
真正注册且可用的是 `GET/PUT /api/admin/config`（实测 200，返回 base_url / model /
max_context_tokens / max_output_tokens / protocol），但它的字段在**模型页**配置，
管理页的「实例设置」展示的是 `default_soul` / 配额 / 网络策略——**那些确实没有后端**，
是诚实的骨架页。

---

## 登录线（2026-10-06 收尾）

用户定的范围：**只打通「登录 / 退出」这条线。注册默认关闭；注销（账号删除）与
验证系统（验证码 / 邮箱）不做。** 下面记清楚「做了什么」和「刻意没做什么」。

### 做了什么

| 端点 | 状态 | 备注 |
|---|---|---|
| `GET /api/setup/status` | 公开 | `{ setup_required, user_count, registration_enabled:false }` |
| `POST /api/setup/initial-admin` | 公开 | 201；users 表非空后永久 409 |
| `POST /api/auth/login` | 公开 | `{ access_token, token_type, expires_in, expires_at, user }` |
| `POST /api/auth/refresh` | **需令牌** | 轮换，旧令牌立刻 401 |
| `POST /api/auth/logout` | 需 Bearer 头 | 200 + `{ revoked, revoked_count, note }`，幂等 |
| `GET /api/auth/me` | 需令牌 | 回真实用户名，不是 hex id |

契约照抄 Octop `dashboard/src/api/modules/auth.ts:8-23`。
后端 18 项 HTTP 集成测试（`crates/quill-server/tests/auth_http.rs`）全过。

### 刻意没做（以及为什么不抄）

Octop 登录那一套里这些 quill **后端一个都没有**，抄过来就是恒定失败的字段：

- **captcha_token** — 没有验证码系统。
- **OIDC / OAuth / LDAP** — 没有外部身份源。
- **忘记密码 / 改密码** — 没有邮件通道。这两个是最容易被「先画个界面」骗过去的，
  没有邮件就等于点了必然失败，所以不画。
- **`/api/auth/register`** — 注册是关的，路由**根本不存在**（实测 404）。
  建号只有两个入口：首管引导（仅全新实例）与 `QUILL_PASSWORD_USERS`
  （headless 部署用的配置项，**不是注册的替代品**）。
- **账号注销（删除账号）** — 不在本轮范围。

### 三个刻意的设计选择

1. **防用户名枚举**：口令错与用户不存在返回**逐字相同**的 401。
   有测试钉住（逐字比较响应体）。
2. **`logout` 故意不挂 `AuthUser` 提取器**。挂了的话令牌一被吊销，第二次请求
   在进处理器前就被 401 拦掉，`revoked:false` 这条分支永远走不到。
   代价是无法用该端点探测令牌有效性，措辞已如实区分。
3. **登出不影响环境变量令牌**（改配置 + 重启才收回），这点写在响应的 `note` 里。

### 限流（本轮新增）

`crates/quill-server/src/ratelimit.rs`：进程内滑动窗口，按
`(用户名, 来源 IP)` 分桶，15 分钟内 5 次失败即锁。

**为什么控制面自带的账号锁定不够**（这才是加它的理由）：

- 用户名不存在时走 `burn_equivalent_time()` 分支，**没有任何账号可锁**，
  攻击者拿随机用户名可以无限触发 PBKDF2，顺便做用户名枚举。
- 锁定检查（`locked_until`）排在 `verify_stored()` **之后** ——
  账号已经锁死时，每次尝试照样烧满一遍 PBKDF2。

所以限流检查必须发生在**进 `DbBridge` 之前**。有测试
（`a_throttled_login_never_reaches_the_password_hasher`）用「失败计数不再增长」
证明被挡下的请求确实没进 PBKDF2。

**`X-Forwarded-For` 默认不信**（`QUILL_TRUST_PROXY` 默认关闭）。那个头是客户端
自己写的，直接信它，攻击者换个假 IP 就能把限流清零。

**一个刻意的取舍**：被限流时**正确的口令也进不来**。是否放行不能取决于口令是否
正确，否则「猜中一次清空额度」等于给爆破发通行证。有测试钉住这个语义。

限流状态**不落库**（进程内，重启清空）——账号级的持久锁定由 `quill-control` 负责；
把失败计数写进库，攻击者只需刷失败次数就能把库撑大，是用一个 DoS 换另一个。

### 顺手修掉的两个真 bug

1. **token-only 账号用口令登录返回 500**。`ensure_token_user` 写的
   `password_salt` 是哨兵字符串 `"token-only"`（10 字节），而
   `find_credentials` 在看 `password_algo` **之前**就无条件要求 16 字节 salt
   → 撞不变量 → 500「内部不变量被破坏」。正常登录失败被报成服务端故障，
   还会被当成一次失败尝试计入限流额度。修法：先看 algo，`token-only` 用零值摘要。
2. **`api/client.ts` 解析错了错误信封**。后端发的是嵌套的
   `{error:{code, detail, next_step}}`，前端按顶层 `{code, message}` 找 ——
   于是**全站**每个接口报错都退化成 `Request failed (500)`，
   后端写的中文说明与「下一步：…」一条都到不了用户眼前。

### 已知不足

- 登录限流是**单进程**的。多实例部署时每个进程各算各的额度。共享额度需要
  外部存储（Redis 之类），本轮没做。
- `POST /api/auth/register` 返回的是**通用 404**（`本实例没有路由 …`），
  不是 403。对外隐藏注册通道是故意的，但文案上没说「注册已关闭」。
- `/api/auth/me` 在 profile 读不到时（用户刚被从库里删）会退化成回 hex id，
  并带 `profile_unavailable: true` 说明。属诚实降级，不是静默假装正常。

---

## 对话地基（2026-10-06）

先说结论：**MVP 主干已经通了**。下面几项都是实测（不是读代码推断），
对着本机 llama.cpp + Qwen3.5-4B 跑的。

| 环节 | 实测结果 |
|---|---|
| 登录 | 浏览器完整回路 + 服务端吊销验证 |
| 真实模型 | 1.8s 往返，usage 真实 |
| 专家人格 | `persona_applied: true`，模型以「AI 编程实战导师」身份作答 |
| 多轮上下文 | 隔一轮干扰后仍答出先前的数字 |
| **工具调用** | 模型自主调用 `list_experts`，回灌后给出基于真实结果的回答 |

### 工具调用（`crates/quill-server/src/tools.rs`）

**provider 层本来就是完整的**：`ToolSpec` 带 JSON Schema、`ChatRequest::with_tools`
会把 `tools` 写进请求体、响应侧能从 `delta.tool_calls` 拼装、`Message::tool_result`
也现成。缺的是「有哪些工具」和「调完怎么办」这两段，本轮补的就是它们，
`quill-provider` 一行没改。

**修掉的隐患**：`api_chat.rs` 此前只取 `reply.answer()`，而 `has_answer()` 在
「只有 tool_calls」时也返回 `true`。后果是模型一旦真的要求调工具，**正文会凭空
消失，而 HTTP 照样 200** —— 不报错、不 501，界面上就是一条空消息。现在改成
「执行 → 以 `role=tool` 回灌 → 再问一次」，并设 4 轮上限（每轮都是一次真实模型
调用，不设上限会挂死并烧 CPU）。达到上限仍无正文时**明确报错**并列出已执行的工具，
不静默返回空。

**工具全部只读。** 写文件、跑命令这类能改系统状态的故意不做 —— 一个能被对话
内容驱动的执行器是远程代码执行，不是 Agent 能力。写入类操作等 MCP 接入后由用户
显式配置允许的工具提供。

**按用户过滤**：工具查询走 `list_visible(&uid)`，与 `GET /api/experts` 同一套
可见性规则。否则模型能列出用户界面上看不到的专家，工具就成了绕过隔离的后门。

**前端还没有工具调用 UI**：`api_chat` 已在响应里给出 `tool_calls` 轨迹与
`tool_rounds`，但 `ui/web/src/chat/` 还没有渲染它的地方。

### 团队分工：只有记账

`api_dispatch.rs` 写完台账后**固定返回 `"executed": false`**。不是 501，
是「成功但什么都不做」—— 比 501 更坏的一种坏法。`MemberExecutor` trait 只有
testkit 里的假实现，生产路径为零。`teams` 表的 `guidelines` / `max_dispatch` /
`max_replan` / `max_ask_depth` 四个字段，生产 Rust 代码**零引用**。

「成员相互沟通」在 `quill-domain/src/team.rs` 里**没有对应物**：无 mailbox、
无 handoff、无 mention。`REQUIREMENTS.md` 原文只写了「专家与专家团」六个字，
没有细节 —— 属于**没有需求锚点**的状态，实现前要先定语义。

**这块的 UI 反而是诚实的**：`TeamsTab.tsx` 明确渲染「只记账，未派工」。

### SKILL / MCP：存储层已通，协议层没接（2026-10-06 更新）

**这一节推翻了上一版结论。** 上一版写的是「引擎层压根没写、`mcp_repo` /
`skills_repo` 都不存在、保存按钮必 500」。现在：

- `crates/quill-server/src/mcp_repo.rs` / `skills_repo.rs` 已落地，
  `GET|POST /api/extensions/mcp` 与 `GET|POST /api/extensions/skills`、
  两个 `DELETE /{name}` 都是真实现，不再是 501 桩。
- 0007 迁移把 `mcp_servers` 重建，transport 改为
  `stdio | streamable_http | sse | builtin`，补 `cwd` / `max_concurrent_calls` /
  `enabled_capabilities_json` 三列，老库的 `'http'` 搬迁为 `'streamable_http`。
  **那个「必 500」不再成立。**
- 上面那个「保存按钮必 500」的真 bug 也修了：见下方「run_migration 的连接池 bug」。

**仍然没做的是协议层。** `GET /api/extensions/mcp` 的响应里 `connected` 恒为
`false`，`note` 写明「下一步：接 rmcp 协议层」。`PATCH /api/extensions/mcp/{name}`
与 `GET /api/extensions/plugins` 仍是 501。界面上不许出现任何由 `servers.length`
推断出来的「已连接」字样。

#### 修掉的一个深层 bug：`run_migration` 与连接池

`quill_store::run_migration` 原来逐条 `sqlx::query(s).execute(pool)`，而
`SqlitePool` **每条 query 各自取连接**。于是 0007 开头的
`PRAGMA foreign_keys = OFF` 落在连接 A，紧接着的 `DROP TABLE` / `ALTER TABLE` 却在
连接 B 上照旧开着外键，0007 直接报
`there is already another table or index with this name: mcp_servers`，
`quill doctor` 退出码 2，六个 CLI 用例全红。

**为什么一直没被发现**：`quill-store` 的迁移测试用 `in_memory()`，那条池
`max_connections(1)`，单连接下 `PRAGMA` 永远生效。真实服务是
`configure_pool(path, 5)`，五连接，问题必现。已补
`all_migrations_apply_through_a_multi_connection_file_pool` 钉住。

现在 `run_migration` **固定用一条连接并包在一个事务里**。顺带：失败时错误里带
「第 N/M 条语句失败：… 语句：…」——只说 `error returned from database` 的话，
人得回去自己数 SQL。

#### 指纹（`asset_hash`）

内容没变就整行跳过，不动 `updated_at`；否则每次点保存所有行都像变过一遍，
`updated_at` 就失去意义了。指纹必须**32 字节**（`mcp_servers.asset_hash` 与
`skills.content_hash` 的 CHECK 都是 `length = 32`），所以复用 `db::digest32`
而不是截 SHA-256。分段用「个数 + 每项长度前缀」：`[""]` 与 `[]`、`["a\u{1}b"]` 与
`["a","b"]` 都必须算出不同指纹（这两条都是被自己的单测抓出来的真碰撞）。
`env` 与 `headers` 分两个区段算，合成一张 map 会让同名字段互相覆盖。

#### SKILL 即工具（抄 Octop 的 `SkillListItem.tool_name`）

SKILL 不走 prompt，走 `tools` 字段 —— 于是「SKILL 怎么进 prompt」这个问题
**不必回答**，用不到就不占常驻上下文。正文落盘到 `<db 同级>/skills/{slug}.md`，
`skills` 行只留摘要与路径（与 Octop「挂目录给运行时」一致，也让
`content_hash` 名副其实）。`skills_repo::as_tool_spec` 已经能把它变成
`ToolSpec`，**但还没接进 `ToolRegistry`** —— 那是接上工具调用后的下一步。

#### 参考实现

goose 在 `vendor/goose/` 用 `rmcp` crate，`.mcp.json` 格式
`{ "mcpServers": { name: { command, args, env, cwd } } }` —— 但**只支持 stdio**，
HTTP/SSE 它没有。SKILL 参考 octop：`skills/{slug}/SKILL.md`，且它**不把内容拼进
prompt，是挂目录给运行时按需加载**。

#### 待决 / 下一步

1. 引入 `rmcp` 铺协议层，让 `tools/list` 真能连上（A 部分，用户已同意引入）。
   结论：抄 goose 的**技术选型**（`rmcp` crate），但不复用它的 crate ——
   `vendor/goose` 是独立 workspace（自带 `[workspace]`），quill 无法 path-depend；
   它的 `mcp_client.rs` 有 1687 行且与 agent 类型深度耦合。
2. 把 SKILL 通过 `as_tool_spec` 挂进 `ToolRegistry`，按用户过滤。
3. `PATCH /api/extensions/mcp/{name}`（部分更新）与 `GET /api/extensions/plugins`。

#### 顺带修掉的前后端契约 bug

前端 `<input name="name">` 的 `pattern` 原来是 `[a-z][a-z0-9_]{0,31}`，只允许
下划线；服务端把 `_` 归一成 `-` 并回填 `company-search`。于是
**「保存 → 重新编辑」这一圈直接死掉** —— 编辑框里回填的 `company-search`
过不了页面自己的 `pattern`，浏览器拦下提交，用户只看到一个说不清来由的红框。
旧写法还要求首字符是字母，而服务端允许数字开头。

已把 pattern 改成 `[a-z0-9]([a-z0-9-]{0,62}[a-z0-9])?`（`MCP_NAME_PATTERN`，
`ui/web/src/devices/mcpConfig.ts`），并在 `mcp_repo.rs` 里加了同源表子：
**方向是「服务端产出 ⊆ 前端能收」**，反过来不成立（前端比服务端严一点是提前
拦下，严过头才是 bug）。两个文件里都有对照注释。

`expert.skill_count`（写的是字面量 `0`）与 `expert.tool_policy_json`
（硬编码 `'{}'`）仍是装饰字段，零消费者。

