# octop → quill 迁移功能总清单

> **本文件与配套 CSV 是机器从代码生成的事实，不是转述任何既有文档。**
> 数据来源：`D:\96_CoderWorld\quill\.octop-ref\octop`（sparse 检出，commit `eb280112`）、
> `crates/**`、`ui/web/**`。生成器：`.scratch/gen-octop-inventory.py`（只读上游、不联网）。
> 配套数据：`docs/octop-endpoints.csv`（470 行，逐端点）。
>
> **为什么重做这份清单**：上一批人写的全部管理文档一律不可信（用户原话）。
> 本清单的每一条都指向真实源码位置；任何一条都可以打开对应文件核对。

---

## 0. 最高指示（唯一可信的约束）

> 以 **Rust** 为实现语言（前端 TS 属壳，不受此限）；以腾讯 **octop** 为产品外壳；
> 以 **goose** 为 Agent 内核；面向个人 / 家庭 / 小团队的私人团队 Agent 平台。

判断「要不要迁移」只用这一条：**它是不是这个壳里该有的能力？**
判断「怎么迁移」只用这一条：**能不能用 goose 的手段做，用 Rust 实现？**

---

## 1. 规模事实（代码测量，非文档陈述）

| 维度 | 实测值 | 出处 |
|---|---|---|
| octop 后端路由文件 | 55 个 `.py`，20 970 行 | `find .octop-ref/octop/src/octop/api/routers -name '*.py'` |
| octop 后端端点数 | **470 条**（含 8 个 WS） | `docs/octop-endpoints.csv` |
| octop 前端文件 | 735 个 `.ts/.tsx`，128 151 行 | `find .octop-ref/octop/dashboard/src` |
| quill 工作区 crate | 12 个 | `crates/*/Cargo.toml` |
| quill 注册路由 | **69 条 `.route(`**，其中 `not_implemented(` 桩 **14 处** | `crates/quill-server/src/routes.rs` |
| quill 迁移 | 10 个（0001–0010） | `crates/quill-store/migrations/` |
| quill 前端 | 114 个 `.ts/.tsx`，25 328 行 | `find ui/web/src` |
| quill Rust 测试 | **1266 passed / 0 failed** | `cargo test --workspace`（2026-10-08 实跑） |

### crate 依赖真相（从各 `Cargo.toml` 读出）

```
quill-adapters  ← agent / control / server / upgrade / wiki     （最底层契约）
quill-domain    ← server
quill-store     ← cli / server / upgrade
quill-control   ← cli / server
quill-agent     ← cli / server
quill-wiki      ← cli / server
quill-provider  ← server
quill-backup    ← cli / server
quill-upgrade   ← cli（**声明了依赖但 src 里零调用**）→ 见 §7
quill-testkit   ← 仅 dev-dependency
```

---

## 2. 分类口径（回答「哪些是前端效果、哪些要打通 goose 核心」）

| 分类 | 判据 | 条数 |
|---|---|---|
| **goose核心** | 该能力必须落到 Agent 执行内核（对话 / 工具 / MCP / provider / 子 agent / 记忆 / 上下文 / 流式 / 技能） | **198** |
| **平台** | 用户 / 权限 / 管理 / 备份 / 升级 / i18n / 设置 | 121 |
| **外部依赖** | 依赖第三方服务或浏览器自动化（扫码 / OAuth / Ollama / 媒体 / 语音 / 远程桌面） | 100 |
| **数据** | 知识库 / 资料库 / 文件 / 存储 | 51 |
| **前端效果** | 迁移过来主要是界面 / 交互（渲染、布局、表单、图表、组件） | 见 §5（前端 735 文件） |

> 一个能力可以两者兼有：后端端点属 **goose核心**，它的展示层属 **前端效果**。
> §4 列后端（按核心与否），§5 列前端（纯界面）。

---

## 3. 「必须打通 goose 核心」清单（198 条端点，迁移的重头）

这一批是**在 Rust 里重建、且必须与 goose 内核语义对齐**的部分。
每行 = 一个 octop 端点；「quill 现状」以 `crates/**` 源码为准。

### 3.1 对话内核（chat/*）

| octop 端点 | 用途 | quill 现状 |
|---|---|---|
| `WS /api/agents/{id}/chat/ws` | 聊天主通道（一次 turn，双向流） | **部分**：`POST /api/sessions/{id}/messages/stream`（SSE）已实现，**WS 未做** |
| `POST /api/agents/{id}/chat/hitl/resume` | 恢复人工审批（HITL） | 未做 |
| `POST /api/agents/{id}/chat/polish` | 润色提示词 | 未做 |
| `GET /api/agents/{id}/chat/welcome` | 欢迎快捷卡片 | 未做 |
| `GET/POST /api/agents/{id}/threads` | 会话列表 / 新建 | **已实现**（quill 叫 `sessions`） |
| `GET /api/agents/{id}/threads/{tid}/history` | 会话历史 | **已实现**（`GET /api/sessions/{id}/messages`） |
| `GET /api/agents/{id}/threads/{tid}/context-usage` | 上下文用量 | **已实现**（`GET /api/sessions/{id}/context`） |
| `POST /api/agents/{id}/threads/{tid}/fork` | 分叉会话 | 未做 |
| `GET .../threads/{tid}/history/export` | 导出历史 | 未做 |
| `PATCH/DELETE .../threads/{tid}` | 更新 / 删除会话 | **部分**：DELETE 已实现；PATCH **未注册**（405） |
| `GET .../threads/{tid}/trajectory`（+ events/metrics/stream/export） | 运行轨迹 | 未做 |
| `WS /api/notifications/ws` | 全看板通知 | 未做 |

### 3.2 Agent（专家）生命周期与配置

| octop 端点 | 用途 | quill 现状 |
|---|---|---|
| `GET/POST /api/agents` | 列出 / 创建 | **已实现**（`/api/experts`） |
| `GET/PATCH/DELETE /api/agents/{id}` | 读取 / 更新 / 删除 | **已实现** |
| `POST /api/agents/{id}/start|stop|reload` | 启停 / 重载 | 未做（quill 无 agent 进程概念） |
| `GET /api/agents/{id}/status` | 运行时状态 | 未做 |
| `GET/PUT/PATCH /api/agents/{id}/tool-settings` | 工具开关 | **部分**：`/api/extensions/skills` 有 `enabled`；**内置工具 deny 列表未做** |
| `GET/PUT /api/agents/{id}/heartbeat-config` | 心跳配置 | 未做 |
| `GET/PUT /api/agents/{id}/proactive-care` | 主动关怀 | 未做 |
| `GET/PUT /api/agents/{id}/envs`（全局 `envs`） | 环境变量 | 未做 |
| `POST /api/agents/{id}/avatar` | 头像 | 未做 |

### 3.3 工具 / MCP / Connector

| octop 端点 | 用途 | quill 现状 |
|---|---|---|
| `GET/POST /api/extensions/mcp…`（等价物） | MCP 服务器配置 | **已实现**：`GET/POST/DELETE /api/extensions/mcp`；`PATCH` 是 **501 桩** |
| `…/connectors/catalog`、`/connector-instances` | Connector 目录与实例 | 未做（quill 无目录层） |
| `POST/GET/DELETE /api/internal/mcp/{kind}/{id}` | 托管 connector 的 MCP 网关 | 未做 |
| `GET /api/slash/commands` | 斜杠命令目录 | 未做 |
| `PUT/PATCH /api/agents/{id}/tool-settings` | 工具启停 | **部分**（见 3.2） |

### 3.4 Skills / Skill 包

| octop 端点 | 用途 | quill 现状 |
|---|---|---|
| `GET/POST/PUT/DELETE /api/agents/{id}/skills…` | agent 级 SKILL 库 | **已实现**：`/api/extensions/skills` CRUD + `PATCH` 开关 |
| `…/skills/{name}/enable|disable` | 启停技能 | **已实现**（`PATCH`） |
| `…/skills/import`、`/skills/copy` | 导入 / 复制 | **部分**：导入未接线 |
| `…/skill-packages…` | 全局 skill 包 | 未做 |
| `GET /api/skills/hub/search|rankings`、`…/hub/install` | SkillHub | **已实现**：`/api/extensions/skill-hub*` |

### 3.5 记忆（goose 侧最重的一块，quill 零实现）

| octop 端点族 | 用途 | quill 现状 |
|---|---|---|
| `…/memory/atoms|raw_events|entities|episodes|journal|candidates`（list/get） | 记忆看板六类 | **未做** |
| `…/memory/stats/*` | 数量 / 增长 / 类型统计 | 未做 |
| `…/memory/candidates/{id}:promote|reject` | 候选沉淀 | 未做 |
| `…/memory/atoms`（新建 / replace / deprecate） | 原子记忆写操作 | 未做 |
| `…/memory/extract-config` | 抽取配置 | 未做 |
| `…/memory/portable/pack|adopt|doctor` | 跨主机迁移 | 未做 |
| `…/memory/daily…` | 每日记忆文件 | 未做 |

> quill 侧只有一个**只读**的 `ui/web/src/memory/MemoryPage.tsx`，后端 `/api/wiki/*` 只有
> 4 个只读端点（`api_wiki.rs`），`ingest/query/search` 是 501 桩。**记忆 = 全空**。

### 3.6 子 agent / 团队（goose 内核对齐点）

| octop 端点 | 用途 | quill 现状 |
|---|---|---|
| `GET /api/subagent-catalog`（+ `/divisions`、`/{slug}`） | 内置子 agent 目录 | 未做 |
| `GET/POST /api/agents/{id}/subagents…` | agent 子 agent | 未做 |
| `GET/POST/PATCH/DELETE /api/teams…` | 专家团 CRUD | **已实现**（`api_teams.rs`） |
| 团队派工真执行 | — | **已实现（第一版）**：`ProviderMemberExecutor` + `POST /api/teams/{id}/dispatch/run`（`api_dispatch::run`）。`steer`/`abort` 未做；成员产出未落库 |

> 参照实现：`vendor/goose/crates/goose/src/agents/subagent_handler.rs` 的
> `run_subagent_task`（每子 agent 独立 config + 独立 session）、
> `…/platform_extensions/summon.rs` 的 `delegate` / `load`。

### 3.7 Provider / 模型

| octop 端点 | 用途 | quill 现状 |
|---|---|---|
| `GET /api/providers`、`/presets`、`/resolved`、`/active-model` | 用户侧 provider | **部分**：`api_providers.rs` 是 admin 侧 |
| `POST/PATCH/DELETE /api/admin/providers` | 管理 provider | **已实现**（`api_providers.rs`，admin-only） |
| `POST /api/admin/providers/test-draft`、`/fetch-models` | 测试 / 拉模型 | **部分**：`GET …/{id}/models` 真探测；test-draft 未做 |
| `POST /api/admin/providers/codex-oauth/*` | ChatGPT 设备登录 | 未做 |

### 3.8 定时任务 / 终端 / 工作区 / 上传

| octop 端点 | 用途 | quill 现状 |
|---|---|---|
| `GET/POST/PATCH/DELETE /api/agents/{id}/cron…` | 定时任务 | **未做**（`/api/cron` 连路由都没有） |
| `GET …/terminal/context`、`WS …/terminal/ws` | 交互式 PTY 终端 | 未做 |
| `…/workspace/tree|file|mkdir|move|upload|download|glob|grep|archive` | 工作区读写 | **未做**（前端工作区页也没有真接口） |
| `POST /api/agents/{id}/upload` | 聊天附件 | 未做 |
| `GET …/media/preview` | 媒体预览 | 未做 |

### 3.9 远程实例协作（octop 的「远程 octop 协作」= 原始需求 3）

| octop 端点 | 用途 | quill 现状 |
|---|---|---|
| `POST /api/bridge/probe`、`/bridge/connections…` | 远程实例连接 | 未做 |
| `WS /api/bridge/ws` | 跨实例转发通道 | 未做 |
| `…/bridge/connections/{id}/agents|providers|knowledge-bases` | 远端资源浏览 | 未做 |

> **对应原始需求第 3 条**（octop 与远程 octop 协作）。quill 侧零实现。

---

## 4. 后端端点全清单（470 条，按文件分组摘要）

> 完整逐条见 `docs/octop-endpoints.csv`。下表是**每个路由文件的汇总**，便于排期。

| octop 文件 | 端点数 | 分类 | 接核心 | quill 现状 |
|---|---|---|---|---|
| `chat/`（routes/history/trajectory/ws/notify_ws） | 29 | goose核心 | 是 | 部分 |
| `memory.py` | 22 | goose核心 | 是 | 未做 |
| `channels.py` | 21 | 外部依赖 | 否 | 部分（仅微信） |
| `skills.py` | 18 | goose核心 | 是 | 部分 |
| `bridge.py` | 17 | goose核心 | 是 | 未做 |
| `workspace.py` | 15 | 数据 | 否 | 未做 |
| `skill_packages.py` | 15 | goose核心 | 是 | 未做 |
| `plugins.py` | 14 | 平台 | 否 | **桩(501)** |
| `agents.py` | 13 | goose核心 | 是 | 部分 |
| `experts.py` | 13 | 平台 | 否 | 部分 |
| `users.py` | 12 | 平台 | 否 | 部分 |
| `setup.py` | 11 | 平台 | 否 | 部分 |
| `backup.py` | 11 | 平台 | 否 | 部分 |
| `acp.py` | 11 | goose核心 | 是 | 未做 |
| `filesystem.py` | 9 | 数据 | 否 | 未做 |
| `auth.py` | 8 | 平台 | 否 | 已实现 |
| `cron.py` | 8 | goose核心 | 是 | 未做 |
| `security.py` | 7 | 平台 | 否 | 未做 |
| `mbti.py` | 7 | 平台 | 否 | **已实现** |
| `providers.py` | 11 | goose核心 | 是 | 部分 |
| `subagents.py` | 5 | goose核心 | 是 | 未做 |
| `teams.py` | 6 | goose核心 | 是 | 部分 |
| `usage.py` | 4 | 平台 | 否 | 已实现 |
| `voice.py` | 12 | 外部依赖 | 否 | 未做 |
| `connectors.py` | 31 | goose核心 | 是 | 部分 |
| `knowledge_bases.py` | 26 | 数据 | 否 | 未做（quill 用 quill-wiki 替代） |
| `browser/*` | 13 | 外部依赖 | 否 | 未做 |
| `desktop/*`、`mobile/*` | 12 | 外部依赖 | 否 | 未做 |
| `ollama_models.py`、`onnx_models.py`、`search.py` | 16 | 外部依赖 | 否 | 未做 |
| `auth_ldap/oauth/oidc.py` | 20 | 外部依赖 | 否 | 未做 |
| `media_generation.py`、`observability.py`、`tls.py` | 14 | 外部依赖 | 否 | 未做 |
| 其余（admin/i18n/invites/preferences/proactive_care/settings/slash/storage_backends/terminal/uploads/internal_mcp/agent_files/agent_tools/envs/health/update/update_store/ollama_download_store） | ~90 | 平台/goose核心 | 混合 | 多数未做 |

**合计：goose核心 198 · 平台 121 · 外部依赖 100 · 数据 51 = 470。**

---

## 5. 前端效果清单（735 文件，按页面/组件归类）

> 纯界面层，不改动 Rust 内核语义。quill 侧对应物在 `ui/web/src/**`。

### 5.1 路由（octop `dashboard/src/routes/index.tsx`）

| octop 路径 | 组件 | quill 对应 |
|---|---|---|
| `/chat`、`/chat/:agentId`、`/chat/:agentId/:threadId` | Chat（全屏） | **已实现**：`/chat`、`/chat/:sessionId` |
| `/experts` | 专家管理（我的/团队/库/市场 4 tab） | **已实现**：`/experts` |
| `/personalization/*` | 个性化 7 页签 | **已实现**：`/personalization`（含 MBTI） |
| `/connectors` | 连接器目录 | 未做 |
| `/agent-config` | Agent 高级配置 | 未做（部分并入专家表单） |
| `/acp` | ACP runner 管理 | 未做 |
| `/tasks` | 定时任务 | 未做（quill 的 `/automations` 配的是**假后端**） |
| `/skill-packages` | 技能包 | 未做 |
| `/knowledge-bases` | 知识库 | 未做 |
| `/bridge` | 桥接连接 | 未做 |
| `/token-usage` | Token 用量 | **已实现**：`/usage` |
| `/workbench/terminal|/browser` | 终端 / 浏览器工作台 | 未做 |
| `/remote-desktop/*`、`/remote-phone` | 远程桌面 / 手机 | 未做 |
| `/admin/models|users|backend|plugins|advanced|security|sso` | 管理区 | **部分**：quill 有 `/admin/models|instance|backup|users` |
| 大量 legacy 重定向（`/orca/*`、`/octop/*` 等） | 兼容旧链接 | 不迁移 |

### 5.2 页面功能（`pages/`）

| 页面 | 关键交互（前端效果） | quill 对应 |
|---|---|---|
| `Chat/index.tsx`（1792 行） | 会话侧栏 / 消息虚拟滚动 / 输入器 / Dock 面板（文件·浏览器·终端·工具UI·知识引用）/ 轨迹抽屉 / HITL 卡片 | **部分**：`chat/ChatPage.tsx` 有会话 + 消息 + 流式；**Dock / 轨迹 / HITL / picker 未做** |
| `Experts/index.tsx`（917 行） | 卡片/表格切换、创建/编辑/删除、团队编组、模板库、市场、发布 | **部分**：`experts/` 有我的/团队/库/市场 |
| `Agent/Personalization/` | 7 页签聚合（skills/subagents/tools/plugins/mbti/memory/channels） | **部分**：`personalization/PersonalizationPage.tsx` 已有页签壳 |
| `Agent/Memory/MemoryPanel` | 记忆 9 页签（概览/画像/记忆树/情绪日记/候选/整理/对话/主动关心/设置） | **未做** |
| `Agent/Channels/ChannelsPanel` | 渠道卡片 + 二维码绑定 | **部分**：`channels/Channels.tsx`（仅微信） |
| `Agent/Connectors` | 目录 / 实例 / OAuth / CLI 安装 | 未做 |
| `Agent/Skills` | 已装/内置/市场/技能包 4 子 tab | **部分**：`skills/` 有页面 + Hub |
| `Agent/Tools` | 内置/插件/ACP 3 tab | **未做** |
| `Agent/ACP` | ACP runner 卡片 | 未做 |
| `Agent/Workspace` | 文件树 + 编辑 + 上传/下载 | **未做**（quill 工作区页无真接口） |

### 5.3 可复用组件（`components/`，迁移价值最高的公共件）

| octop 组件 | 用途 | quill 对应 |
|---|---|---|
| `ChatPicker/SearchablePickerPanel` | 通用可搜索选择面板 | 未做（quill 用自研 picker） |
| `Markdown/` | GFM + 高亮 + mermaid + 数学 + **流式稳定** | **部分**：`chat/Transcript.tsx` |
| `ResizableTable`、`Skeleton`、`EmptyState`、`ErrorBoundary` | 表格/骨架/空态/错误边界 | **部分**：quill 有 `components/Page.tsx` |
| `StreamConnectingIndicator`、`StreamSetupGuide`、`StreamEdgeControls` | 流状态 HUD | 未做 |
| `PwaInstallPrompt`、`PwaUpdatePrompt` | PWA | 未做 |
| `BrowserViewer`、`BrowserWorkspace`、`ChromeTabBar` | 远程浏览器 | 未做 |
| `DesktopWindowControls` | 桌面壳窗口控件 | 未做（无桌面壳） |
| `CatalogTypeCard`、`AppVersionBadge`、`CurrentVersionBadge`、`BetaBadge` | 目录卡 / 版本徽章 | 未做 |
| 其余约 30 个（AuthGuard / AuthImage / AvatarDropdown / LanguageSwitcher / ThemeSwitcher / EmojiPicker / ExpertColorPicker / PdfDocumentPreview / TodoListInline / TokenCountInput …） | 各类基础件 | **部分**：quill 有 auth/i18n/theme |

### 5.4 hooks（octop `hooks/`，约 40 个）

流相关：`useBrowserStream`、`useDesktopStream`、`useMobileStream`、`useAgentThreadChat`、
`useDashboardPushToast`、`useEmbeddingDownloadWS`。
界面相关：`useCardTableView`、`usePathTabs`、`useFilteredList`、`useHorizontalResize`、
`useIsMobile`、`useCurrentUser`、`useUpdateStatus`、`useVoiceInput/Output` 等。
→ quill 对应物在 `ui/web/src/**` 内部 hooks，**未做系统化对齐**。

---

## 6. 已完成标记（quill 现状汇总，以代码为准）

| 状态 | 含义 | 端点数 |
|---|---|---|
| **已实现** | 有真实现（读/写库或真调上游） | 18 |
| **部分** | 核心路径通了，但子能力缺 | 193 |
| **桩(501)** | 路由登记但要 501（诚实标注，不假成功） | 20 |
| **未做** | 无对应实现 | 238 |
| **—** | 无端点（聚合/helper） | 1 |

### quill 侧**已实现**清单（代码可查，共 18 项能力域）

- 认证：`GET /api/setup/status`、`POST /api/setup/initial-admin`、`POST /api/auth/login`、
  `POST /api/auth/refresh`、`POST /api/auth/logout`、`GET /api/auth/me`（`api_auth.rs`）
- 专家 CRUD：`api_experts.rs`（+ `general_expert.rs` 保证每用户一份「通用专家」）
- 专家团 CRUD：`api_teams.rs`
- 派工记账：`api_dispatch.rs`（**只记账，无消费者**）
- 会话与消息：`api_chat.rs`（含 `POST …/messages/stream` SSE 流式，`api_chat_stream.rs`）
- MCP：`api_extensions.rs` + `mcp_client.rs`（rmcp 真拉 stdio 子进程、真握手、`tools/list`、`tools/call`）+ `mcp_repo.rs`
- SKILL：`api_extensions.rs` + `skills_repo.rs` + `tools.rs`（挂进对话工具表）
- SkillHub 市场（技能包 + 单技能）：`skillhub.rs`、`skillhub_unpack.rs`
- 专家市场：`api_expert_market.rs`
- 多供应商：`api_providers.rs` + `llm_providers.rs`（真探测 `/models`）
- MBTI：`api_mbti.rs` + `mbti/*.rs` + 迁移 0010
- 通道：`api_channels.rs` + `channels/{store,weixin}.rs` + 迁移 0009（**仅微信**）
- 备份：`api_backup.rs` + `quill-backup`（export/verify 真算 sha256；restore 进程内不做，给 CLI 命令）
- 用量：`GET /api/usage` + `session_metrics.rs`
- 用户管理：`GET /api/users`、`PATCH /api/users/{id}`（`api_users.rs`）
- 实例配置：`GET/PUT /api/admin/config`（`api_admin.rs`）
- 健康：`GET /healthz`
- 资料库（只读）：`api_wiki.rs` 四端点 + `quill-wiki`

### quill 侧**诚实标注为未接通**的 501 桩（14 处，`routes.rs`）

`POST /api/experts/import`、`GET /api/experts/export`、`POST /api/wiki/{ingest,query,search}`、
`PATCH /api/extensions/mcp/{name}`、`GET /api/extensions/plugins`、
`POST /api/extensions/bundle/import`、`GET /api/extensions/bundle/export`、
`GET /api/upgrade/{check,history}`、`POST /api/upgrade/prepare`、`GET /api/ws`。
（另有 `POST /api/users`、`DELETE /api/users/{id}` 两条走专用函数返回 501，**有意不做**。）

---

## 7. 代码里发现的真实缺陷（与迁移无关，但要修）

这些**不是迁移项**，是当前代码/机制本身的洞。以代码为准列出：

1. **`scripts/status.mjs` 的 `selfCheck()` 不校验 `kind` 合法性。**
   `decide()` 只认 `test/cmd/absent/manual`；`project/items.mjs` 里 `B6-4` 用了
   `kind: 'script'`、`B6-6` 用了 `kind: 'auto'` → 两条判据永久落在 `BROKEN`（真实状态未知）。
   而 `selfCheck()` 里没有「未知 kind」这一支，**所以自检报「全部成立」，抓不到它**。
   这正是本项目最看重的「判据本身坏了」那一类。
2. **`project/items.mjs` 的 `B2-2` 判据与标题矛盾。**
   标题「成员执行器存在（子 agent 真被执行）」，`verify` 却是
   `{ kind: 'absent', path: 'crates/quill-server/src/member_executor.rs' }` ——
   文件**不存在**时报「已验证」。即：一个说「存在」的条目，靠「不存在」通过。
3. **`vendor/openoctopus-frontend` 的身份**：`UPSTREAM.md` 声称它只是 `index.css` 来源，
   与 Octop 无关。**以代码为准**：`ui/web/src/index.css` 有 sha256 门禁钉着它的哈希，
   那棵树本地**没有 `.git`**（`git -C vendor/openoctopus-frontend` 会往上找到仓库根）。
   → 结论：它确实只贡献 CSS；但「门禁钉哈希」这件事要自己打开
   `.vendor-baseline-check.mjs` 核对，不能转述文档。
4. **`quill-upgrade` 被 cli 声明依赖但零调用**：`crates/quill-cli/Cargo.toml:25` 有它，
   `crates/quill-cli/src/**` 里 `grep quill_upgrade` 为空。120 行只有升级前守卫。
5. **`cargo fmt` 从未被采用**：`gates.sh` 与 `.github/workflows/gates.yml` 都不跑 fmt
   （`gates.yml:51` 注释明写「刻意不开 clippy/rustfmt」）。`cargo fmt --all -- --check`
   实测 **481 个文件**有差异。→ 这是一处**有意的空白**，不是回归。

---

## 8. 推进路线（按「先核，再迁」）

按最高指示 + 依赖关系排序，不按 octop 的原顺序：

1. **先修机制洞**（§7 第 1、2 条）：判据坏了比功能没做更危险。
2. **M2 核心：派工真执行**（§3.6）—— 参照 `vendor/goose` 的 `run_subagent_task`，
   让 `api_dispatch.rs` 的台账真正被消费者消费。这是「专家团」从「摆件」变「能干活」的分界。
3. **对话内核补齐**（§3.1）：HITL、轨迹、会话分叉、WS 主通道。
4. **上下文与记忆**（§3.5）：这是 goose 最有价值、quill 完全空白的一块。
5. **工作区 / 终端 / 上传**（§3.8）：让界面上那些「点了没反应」的入口真通。
6. **远程协作**（§3.9）：对应原始需求 3，工作量最大，放后。
7. **外部依赖类**（浏览器/桌面/手机/语音/Ollama）：按需，不是核心。

---

## 附录

- `docs/octop-endpoints.csv` —— 470 条端点逐行（序号/文件/方法/路径/用途/分类/接核心/quill现状/备注）。
  **文件尾部有两段便于一眼看进度的汇总**：
  - `—— 小计 ——`：按分类 / 按是否接 goose 核心 / 按 quill 现状 / 合计，以及**生成时间**与上游提交；
  - `—— quill 侧已实现 ——`：逐条列出已实现的能力域与其落点文件（判断，非测量）。
- 生成器：`scripts/gen-octop-inventory.py`（只读上游、不联网）
- 复现命令：`python scripts/gen-octop-inventory.py`（重跑会刷新小计与生成时间；
  「分类 / quill 现状」写在生成器的 `FILE_MAP` 里，是人工判断，改它再重跑即可）
