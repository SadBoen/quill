# CODE-TRUTH —— 以代码为准的事实总账

> **这份文件是重建后的管理文档核心。** 判断依据只有一条：**打开源码看它到底写了什么**。
> 上一批人写的 README / MILESTONES / BACKLOG / WORKING / UPSTREAM / UPSTREAM-USAGE /
> `project/items.mjs` / `docs/adr` 一律**不可信**（用户明说），本文件不转述其中任何一句。
> 每条事实后面都给了**复现命令**；对不上就以你跑出来的为准，改这份文件。

---

## 0. 最高指示（唯一可信的约束）

> **Rust 为实现语言**（前端 TS 属壳，不受此限）；**腾讯 octop 为产品外壳**；
> **goose 为 Agent 内核**；面向个人 / 家庭 / 小团队的私人团队 Agent 平台。

它回答两个问题：**该不该有这个能力**（壳里该不该有）、**怎么做**（用 goose 的手段、用 Rust 写）。

---

## 1. 上游 pin（打开文件读出来的，不是抄文档）

| 上游 | 版本 | commit | 出处（读哪一行） |
|---|---|---|---|
| goose | **v1.53.0** | `76da81cb964b21cd096db739302329b40c2998b8` | `vendor/goose/Cargo.toml:11` + `.upstream-pin` |
| octop | **1.0.2b6** | `eb28011249c02cafd389b2d424294c6c1b9cf422` | `git -C .octop-ref/octop rev-parse HEAD` |

```bash
grep -m3 '^version' vendor/goose/Cargo.toml        # → version = "1.53.0"
cat .upstream-pin                                   # → v1.53.0 <sha1>
git -C .octop-ref/octop rev-parse HEAD              # → eb280112...
```

`vendor/goose` 与 `.octop-ref/octop` **都不入库**（gitignore），fresh clone 上没有。
取回：`bash .scripts/fetch-vendor.sh`。

---

## 2. 工作区结构（12 个 crate，行数是 `wc -l` 真值）

| crate | src 行 | 测试行 | 依赖（读 Cargo.toml） | 一句话（读 lib.rs 得出） |
|---|---|---|---|---|
| `quill-adapters` | 1706 | 0 | — | 最底层契约：ID 类型、`MemberExecutor` trait、`KnowledgeBackend` |
| `quill-domain` | 537 | 0 | adapters | 领域模型：`Team`、`TeamId` |
| `quill-store` | 1407 | 995 | — | SQLite 池 + 迁移引擎 + 结构摘要 |
| `quill-control` | 4022 | 1313 | adapters | `ControlPlane`：认证、会话、邀请、口令哈希 |
| `quill-agent` | 2894 | 903 | adapters | `Dispatcher` / `ExpertRegistry` / `MemberExecutor` 泛型包装 |
| `quill-wiki` | 3839 | 702 | adapters | 资料库（LLM-WIKI 变种）：Page/Index/LinkGraph/ingest/query |
| `quill-provider` | 3227 | 481 | — | Provider 抽象 + OpenAI 兼容实现 + SSE 流式泵 |
| `quill-backup` | 1641 | 571 | — | 备份/还原 + manifest + sha256 |
| `quill-upgrade` | **120** | 233 | adapters, store | 只有「升级前先备份」守卫 |
| `quill-testkit` | 1584 | 547 | — | 仅测试用：mock executor、各类计数器 |
| `quill-cli` | 879 | 234 | store,control,wiki,agent,backup,**upgrade** | CLI：doctor/experts/wiki/backup/restore |
| `quill-server` | **26616** | 12845 | adapters,domain,store,control,agent,wiki,provider,backup | HTTP 服务（巨石化） |

```bash
for d in crates/quill-*/; do echo "$(basename $d): $(cat $(find "$d/src" -name '*.rs') | wc -l)"; done
```

**两处依赖真相**（容易被文档说错）：

- **`quill-server` 不依赖 `quill-upgrade`**（`crates/quill-server/Cargo.toml` 无该行）→ `/api/upgrade/*` 三条路由是 501 桩。
- **`quill-cli` 声明了 `quill-upgrade` 却零调用**：`grep -rn quill_upgrade crates/quill-cli/src` 为空。

---

## 3. HTTP 层真相（`crates/quill-server/src/routes.rs`）

```bash
grep -cE '\.route\(' crates/quill-server/src/routes.rs          # → 69
grep -cE 'not_implemented\(' crates/quill-server/src/routes.rs  # → 14
```

- 全仓**唯一**的路由注册处就是 `routes.rs`（`ui.rs` 只做 SPA 静态兜底）。
- 非 `/api*` 未知路径 → SPA；`/api` 下未知路径 → 404 JSON；方法不匹配 → 405。
- `not_implemented(method, path)` → 统一 501 + 中文建议。**14 处桩**：
  `POST /api/experts/import`、`GET /api/experts/export`、
  `POST /api/wiki/{ingest,query,search}`、`PATCH /api/extensions/mcp/{name}`、
  `GET /api/extensions/plugins`、`POST /api/extensions/bundle/import`、
  `GET /api/extensions/bundle/export`、`GET /api/upgrade/{check,history}`、
  `POST /api/upgrade/prepare`、`GET /api/ws`。
- 另有两条**有意** 501（走专用函数，不是 14 处之一）：
  `POST /api/users`（`api_users::create_not_allowed`）、`DELETE /api/users/{id}`（`delete_not_allowed`）。

### 服务端模块（`crates/quill-server/src/`，行数为真值）

| 模块 | 行 | 职责 | 生产调用 |
|---|---|---|---|
| `api_chat.rs` | 2041 | 会话与消息核心（含工具循环 `run_turn`） | 是 |
| `api_extensions.rs` | 1737 | MCP + SKILL + 技能市场 | 是 |
| `skillhub.rs` | 1715 | SkillHub 上游客户端 | 是 |
| `mcp_client.rs` | 1182 | MCP 协议层（rmcp，真拉 stdio 子进程） | 是 |
| `tools.rs` | 1147 | 对话内置工具表 + skill/mcp 挂载 | 是 |
| `llm_providers.rs` | 942 | 多供应商持久化 + 真探测 `/models` | 是 |
| `skillhub_unpack.rs` | 878 | zip 解包 + 安全上限 | 是 |
| `api_channels.rs` | 828 | 通道 REST + 微信扫码 | 是 |
| `error.rs` | 648 | `ApiError` + HTTP 映射 | 是 |
| `mcp_repo.rs` | 660 | MCP 配置存储 | 是 |
| `dispatch_ledger.rs` | 609 | 派工记账落库 | 是 |
| `auth.rs` | 528 | 令牌解析（含环境变量令牌回库核身份） | 是 |
| `pathsafe.rs` | 574 | 唯一的路径包含判定 `is_within` | 是 |
| `ratelimit.rs` | 550 | 登录限流 | 是 |
| `session_metrics.rs` | 452 | 会话级 token 统计 | 是 |
| `channels/{store,weixin}.rs` | 395+595 | 通道存储 + iLink 微信协议 | 是 |
| `mbti/{profiles,questions,score,store,apply}.rs` | 651+422+289+234+206 | MBTI | 是 |
| `api_*`（其余 13 个） | — | 见 §3 路由表 | 是 |

**没有 `member_executor.rs` 这类生产执行器**：
`grep -rnE 'struct [A-Za-z]*Executor' crates/quill-server/src` 为空。
生产侧唯一的 `impl MemberExecutor` 是 `quill-agent/src/dispatch.rs:739` 的泛型
`SharedExecutor<E>` 包装；具体实现只有 `quill-testkit/src/mock_member.rs`（测试用）。

---

## 4. 数据层（`crates/quill-store/migrations/`）

10 个迁移，逐字列出文件名（内容以文件为准）：

```
0001_init.sql                    0006_teams.sql
0002_admin_config.sql            0007_mcp_transport_alignment.sql
0003_llm_providers.sql           0008_token_metrics.sql
0004_expert_persona.sql          0009_channels.sql
0005_expert_source_template.sql  0010_mbti.sql
```

```bash
ls crates/quill-store/migrations/   # → 0001..0010
```

> **纪律**：迁移文件一旦被某个库应用过就**不可再改**（改了会让那条库判成漂移，
> 整条链停住）。要改就新开一条 —— 这条是结构事实，与谁写的无关。

---

## 5. 前端真相（`ui/web/src/`，TS/TSX）

```bash
find ui/web/src \( -name '*.tsx' -o -name '*.ts' \) | wc -l   # → 114
```

**路由**全在 `ui/web/src/app/App.tsx`（`react-router-dom`）：

```
/login
/chat  /chat/:sessionId
/experts   /workspace  /workspace/:workspaceRef   /memory   /usage
/devices   /skills   /automations   /channels   /personalization   /account
/admin/models  /admin/instance  /admin/backup  /admin/users      （RequireAdmin）
重定向：/ → /chat；/mbti → /personalization?tab=mbti；
        /devices/:name → /devices；/admin/mcp → /devices；/admin/settings → /admin/models；* → /chat
```

**页面模块**（每个目录一行）：`app`(路由) `layout`(外壳) `auth`(登录+守卫)
`account` `admin`(实例/用户/备份) `automations` `channels`(微信) `chat`(核心)
`devices`(实为 MCP) `experts`(我的/团队/库/市场) `mbti` `memory` `models`
`personalization` `skills`(含 Hub) `usage` `workspace` `charts`(ECharts) `i18n` `theme`
`components`(Page/collection)。

> **界面诚实基线**：入口凡未接通的，都在 `ui/web/src/capabilityGaps.ts` 与页面文案里
> 标明「缺什么」。**不要把它当成「已实现」** —— 以对应的后端路由为准。

---

## 6. 与上游的实际接法（打开代码看，不转述文档）

| 上游机制 | quill 里的落点（代码） | 形态 |
|---|---|---|
| goose 子 agent 调度协议 | 参考 `vendor/goose/crates/goose/src/agents/subagent_handler.rs`（`run_subagent_task`） | **未接入**：quill 侧无消费者 |
| goose `delegate`/`load` | `vendor/goose/.../platform_extensions/summon.rs` | 只读参考 |
| octop 专家人格（MBTI） | `crates/quill-server/src/mbti/profiles.rs`（16 型）+ `ui/web/src/experts/library/`（17 预设） | **已迁移** |
| octop SkillHub 市场 | `crates/quill-server/src/skillhub.rs` + `api_extensions.rs` / `api_expert_market.rs` | **已接入**（端点真机实测） |
| octop 通道（微信扫码） | `crates/quill-server/src/channels/weixin.rs` + `api_channels.rs` | **已接入**（仅微信；未真机扫码） |
| octop 前端结构 | `ui/web/src/**` | 只读对齐（不要求代码一致） |
| CSS 设计系统 | `ui/web/src/index.css`（有 sha256 门禁） | 来自 `vendor/openoctopus-frontend`（**不是 Octop**，只贡献 CSS） |

```bash
node .provenance-check.mjs    # 逐条核 UPSTREAM-USAGE.md 里的引用是否真的存在
```
> 注意：`.provenance-check.mjs` 核的是 **UPSTREAM-USAGE.md 的记录**，而那份文档本身不可信 ——
> 要判断某条引用真不真，自己打开被引用的那个文件。

---

## 7. 可测规模（当场跑）

```bash
# Rust
cargo test --workspace 2>&1 | grep -E '^test result' | awk '{p+=$4; f+=$6} END {print p, f}'
#   2026-10-08 实测：1266 passed / 0 failed

# 前端（需要 node_modules 是本平台的那份）
cd ui/web && npx vitest run
```

- 本项目**没有固定工具链文件**（无 `rust-toolchain.toml`），实测环境 cargo/rustc **1.99.0**、node **v24.21.0**。
- **`cargo fmt` 从未被采用**：`gates.sh` 与 `.github/workflows/gates.yml` 都不跑 fmt
  （`gates.yml:51` 注释原话「刻意不开 clippy/rustfmt」）。
  `cargo fmt --all -- --check` 实测 **481 个文件**有差异 —— 这是**有意的空白**，不是回归。
- **clippy** 未进门禁，但实跑 `cargo clippy --workspace --all-targets` **退出 0**（仅 warning）。

---

## 8. 代码里查出的真实缺陷（不是迁移项，是本项目自己的洞）

| # | 缺陷 | 证据 | 状态 |
|---|---|---|---|
| 1 | `status.mjs` 的 `selfCheck()` 不校验 `kind` 合法性，`--self-check` 抓不到坏判据 | 已修 | 已修（commit edb8d64） |
| 2 | `items.mjs` B6-4 `kind:'script'`、B6-6 `kind:'auto'` → decide 判「判据坏了」 | 已修 | 已修 |
| 3 | `items.mjs` B2-2「成员执行器存在」绑 `absent` —— 方向写反 | 已修 | 已修 |
| 4 | `quill-cli` 声明 `quill-upgrade` 依赖但零调用 | `grep quill_upgrade crates/quill-cli/src` 空 | 待修 |
| 5 | `quill-server` 是 26616 行巨石 | `wc -l` | 待拆（长期） |
| 6 | `teams` 的 `max_dispatch`/`max_replan`/`max_ask_depth`/`guidelines` 生产侧零引用 | `grep ... crates/*/src` | 待做（M2 派工限额） |
| 7 | `wiki_index`/`skill_count`/`tool_allowlist` 仍在上报，无真实来源 | `grep ... crates/*/src` | 待摘 |

```bash
# 4~7 的复现命令
grep -rn 'quill_upgrade' crates/quill-cli/src
grep -rqE 'max_dispatch|max_replan|max_ask_depth|guidelines' crates/quill-server/src crates/quill-store/src; echo $?
grep -rqE 'wiki_index|skill_count|tool_allowlist' crates/quill-server/src crates/quill-store/src; echo $?
```

---

## 9. 怎么用这份文件

- **要状态**：`node scripts/status.mjs`（机器算出来的，不是读文档）。
- **要「octop 还有什么没迁」**：`docs/OCTOP-MIGRATION-INVENTORY.md` + `docs/octop-endpoints.csv`。
- **要改这份文件**：先跑复现命令，对不上就改它 —— **不要凭记忆写**。
