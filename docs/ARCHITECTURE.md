# quill 架构设计（以代码为准）

> 这份文档只描述**代码里真实存在**的结构。凡结论都附复现命令，你可以当场打脸。
> 仓库里的其它管理文档一律不作为依据（上一批人写的，可能撒谎或过期）。
> 本轮只做**设计**，不改产品契约。落地按 §5 路线图分批走，每批都补测试。

---

## 0. 最高指示与硬约束

唯一可信的约束（用户原话）：

> 以 **Rust** 为实现语言（前端 TS 属壳，不受此限）；
> 以腾讯 **octop**（`.octop-ref/octop`，1.0.2b6 @ `eb280112`）为**产品外壳**；
> 以 **goose**（`vendor/goose`，v1.53.0）为 **Agent 内核**。

### 0.1 打包模型（**先看这条，它决定一切；最高指示第 4 条**）

**最终产物只打包 quill 一个项目。goose 与 octop 都是参考源，不进打包。**

- **抄代码，不依赖项目**。需要 goose 的内核逻辑，就把那段逻辑**移植**进 quill，
  而不是把 goose 加进 `Cargo.toml`。
- **唯一例外**：对方提供的是**可调用的 API 服务**（进程外服务）。goose 是 Rust 库、
  octop 后端是 Python —— 两者都**不是**我们要调的服务。octop 的前端是 TS，可直接搬。
- 因此 `vendor/goose` 与 `.octop-ref/octop` 是**只读的参考书**，随时可删；
  删掉它们后 `cargo build` 必须照常成功。

复现（证明 goose/octop 确实没进依赖）：

```bash
grep -rnE '^\s*(goose|octop)' crates/*/Cargo.toml Cargo.toml          # → 空
```

### 0.2 由此推出的三条架构约束

- **C1 内核 = goose 的移植**：对话循环、工具执行、MCP、provider、子 agent、
  上下文压缩、记忆 —— 这些**属于内核**，quill 应有**忠实移植 goose 的**实现。
- **C2 外壳 = octop 的移植**：用户/鉴权/专家团/技能/渠道/备份/多端同步/前端 ——
  这些是**产品外壳**，参照 octop 在 Rust + TS 里重写。
- **C3 前端只连真后端**：`ui/web` 是 TS，可以照搬 octop 的界面，但不许画假开关。

### 0.3 工程管理方案（最高指示第 5 条）

**门禁驱动的持续交付**（Gate-Driven Continuous Delivery）。本项目是 **Rust 后端 + Web 前端**
的前后端工程，**两侧进同一条流水线、受同一把门禁**：

| 侧 | 门禁 |
|---|---|
| Rust | `cargo build` / `cargo test` / `cargo clippy -D warnings` / `cargo fmt --check` / `cargo deny` / `cargo semver-checks` |
| Web | `npm run typecheck` / `npm run lint` / `npx vitest run` / `npm run build` |

理论根：**部署流水线**（Deployment Pipeline，Humble & Farley《Continuous Delivery》）
＋ **适应度函数**（Fitness Functions，Ford/Parsons/Kua《Building Evolutionary
Architectures》）＋ **ADR**（Nygard）＋ **DORA 四指标**。
一句话：**「能不能交付」由门禁回答，不由人说。**

现状缺口（**2026-10-09 实测复核**）：~~无 `rust-toolchain.toml`~~（**已有**，Q001）、
~~无 `deny.toml`~~（**已有**，Q002）、~~`clippy`/`fmt` 刻意没进 CI~~（**已进**，Q004/Q005，
且 Q107 修好「装 Rust 那一步秒挂」之后 CI 才第一次真跑完）、**无 DORA 度量**（仍未做）。
复现：`ls rust-toolchain.toml deny.toml` 两个都在；
`grep -c "clippy\|fmt" .github/workflows/gates.yml` → 9。
**改这段之前先跑这两条命令** —— 它曾把三条早已补齐的缺口挂在墙上很久。

---

## 1. 现状架构（As-Is）

### 1.1 crate 依赖图（实测）

```
quill-cli ─────────────────────────────────────────────► (顶层，编排一切)
    │
    ▼
quill-server ──► adapters domain store control agent wiki provider backup upgrade
    │                 (2026-10-08 起含 quill-upgrade：`/api/upgrade/prepare`
    │                  真调用它的升级前备份守卫，见 queue Q039)
    │
    ├─ quill-upgrade ──► adapters backup domain store
    ├─ quill-backup  ──► adapters domain store
    ├─ quill-wiki    ──► adapters domain
    ├─ quill-control ──► adapters domain store
    ├─ quill-agent   ──► adapters domain（dev: testkit）（**倒挂已修**，见 §2.4；
    │                     2026-10-08 用上面的命令实测：不再依赖 quill-wiki）
    ├─ quill-core    ──► adapters provider          ← 内核层（Q011 新建，L3）
    ├─ quill-domain  ──► adapters
    └─ quill-provider / quill-store / quill-adapters ──► (叶子，无内部依赖)
```

复现：

```bash
for d in crates/*/; do n=$(basename "$d"); \
  deps=$(sed -n '/^[[]dependencies[]]/,/^[[]/p' "$d/Cargo.toml" | grep -oE 'quill-[a-z]+' | grep -v "^$n$" | sort -u | paste -sd' ' -); \
  echo "$n -> $deps"; done
```

### 1.2 真实分层（按依赖推断）

| 层 | crate | 职责（从代码看） |
|---|---|---|
| L0 基础 | `quill-adapters` | id 类型、UserId/ExpertId/SessionId；无内部依赖 |
| L0 基础 | `quill-store` | 迁移与连接池；无内部依赖 |
| L0 基础 | `quill-provider` | provider 抽象 + 流式 + wire 编码；无内部依赖 |
| L1 领域 | `quill-domain` | Team / Expert 领域模型与规则 |
| L2 应用 | `quill-control` | 用户/角色/凭据/限流 |
| L2 应用 | `quill-wiki` | 知识库 ingest/query/index |
| L2 应用 | `quill-backup` | 备份导出/校验 |
| L2 应用 | `quill-agent` | **专家团编排 / 派工 / 幂等 / 崩溃恢复**（名字骗人，见 §1.4） |
| L3 交付 | `quill-upgrade` | 自升级（2026-10-08 起 `/api/upgrade/prepare` **真调用**它的升级前备份守卫；2026-10-09 起 `POST /api/upgrade/apply` 做**下载 + 校验 + 暂存**并把替换/重启的真命令交出去 —— **进程内自替换仍然不做**，那在 Windows 上被文件锁挡住、在 Unix 上换完内存里也仍是旧代码，属设计上的不可能，见 queue Q063） |
| L4 HTTP | `quill-server` | 路由 + 鉴权 + 序列化 **＋ 内联 SQL ＋ agent 循环**（过载，见 §1.5） |
| L5 CLI | `quill-cli` | 命令行 |

### 1.3 goose 的移植覆盖度（真正的现状）

goose **不是 quill 的依赖**（这符合 §0.1 的意图），所以问题不是"接没接"，
而是**"内核逻辑移植了多少、散在哪里、还缺哪几块"**。

goose 内核的参考源（`vendor/goose/crates/`，实测各 crate 规模）：

| goose crate | 行数 | 职责（Cargo.toml description / 目录） |
|---|---|---|
| `goose-providers` | 15687 | 各家 provider 实现 |
| `goose-provider-types` | 8150 | provider 类型 |
| `goose-local-inference` | 8086 | 本地推理 |
| `goose` | 7473 | 主 crate：`agents/`（`agent.rs` 的 `Agent::reply`、`tool_execution.rs`、`subagent_handler.rs`、`mcp_client.rs`、`prompt_manager.rs`、`state_machine/`、`retry.rs`）、`session/`、`context_mgmt/`、`skills/`、`permission/` |
| `goose-agent` | 1557 | **"The GDK's Agent Loop"**（另一套精简循环） |
| `goose-context-management` | 1156 | **"Conversation compaction for Goose"**（上下文压缩） |
| `goose-sdk` / `goose-sdk-types` | 2831 / 2666 | 可嵌入的 GDK |
| `goose-mcp` | 270 | MCP |

**quill 侧逐块对照（当前）**：

| 内核能力 | goose 出处 | quill 落点 | 覆盖度 |
|---|---|---|---|
| provider 抽象 | `goose-providers`/`-provider-types` | `quill-provider`（叶子，传输）+ `quill-core::llm`（配置与组装，Q015 已搬） | 部分；**组装只剩一套**（2026-10-08） |
| MCP 客户端 | `goose/agents/mcp_client.rs`、`goose-mcp` | **`quill-core/src/mcp_client.rs`**（1213 行，Q014 已搬出 HTTP 层） | 部分（只铺 stdio） |
| 对话循环 | `goose/agents/agent.rs` → `Agent::reply` | **`quill-core/src/turn.rs`**（661 行，Q012 已搬）——壳侧 `api_chat.rs` 只剩「拼 TurnInput + 映射错误 + 落库」 | 部分；状态机已有（`quill-core/src/state_machine.rs`，Q020），**未接线** |
| 工具执行 | `goose/agents/tool_execution.rs` | **`quill-core/src/tools.rs`**（1402 行，Q013 已搬，走 `ToolSources` 端口） | 部分（只读工具，写入类故意不做） |
| 子 agent | `goose/agents/subagent_handler.rs` | `quill-adapters::MemberExecutor` + `member_executor.rs` | 第一版（`steer`/`abort` 未做） |
| **上下文压缩** | `goose-context-management`（1156 行） | **已接线**（Q018）：内核 `quill-core/src/compaction.rs` 是纯逻辑，壳 `quill-server/src/chat_compaction.rs` 注入 provider 与 token 估算；`compaction_threshold_tokens` 真的被读 | 可用（估算口径，无真分词器；删工具响应的重试阶梯因缺可信分类器而不触发） |
| **记忆** | `goose/session` + memory 相关 | **未做** | **未做** |
| 权限 | `goose/permission` | `quill-control`（用户/角色，面向多租户，口径不同） | 需逐块比对 |
| skills | `goose/skills` | `skills_repo.rs` / `skillhub.rs` / `api_extensions` | 部分 |
| slash 命令 | `goose/slash_commands` | 需核 | 需核 |
| session | `goose/session` | `api_chat.rs` 自研 | 部分 |

复现：

```bash
for d in vendor/goose/crates/*/; do n=$(basename "$d"); \
  printf "%-24s %6s\n" "$n" "$(cat "$d"/src/*.rs 2>/dev/null | wc -l)"; done
ls vendor/goose/crates/goose/src/agents/
grep -rniE 'compaction|context.management|记忆|memory' crates/*/src/ | grep -v test
```

**一句话结论**：quill 把 goose 的**接口形状**抄了，但把**内核逻辑塞进了 HTTP crate**；
而 goose 里最重的两块（上下文压缩、记忆）**几乎为零**。

### 1.4 quill-agent 不是内核

`quill-agent` 名字像"agent 内核"，实际只有 2894 行，内容是**专家团领域逻辑**：

```
crates/quill-agent/src/
  1449  expert.rs     专家人格解析 / 校验
   775  dispatch.rs   派工台账（幂等、崩溃恢复、空 waker 自旋 poll）
   637  error.rs      AgentError
```

真正的 agent 循环不在这个 crate 里，而在 HTTP 层 `quill-server/src/api_chat.rs`。
**名字与职责不符**会持续误导人。

### 1.5 巨石：quill-server = 28554 行

> 行数口径：`find crates/quill-server/src -name '*.rs' | xargs wc -l`（2026-10-09 实测）。
> 下面这份清单同一次实测。

`quill-server` 同时承担四件事：**HTTP 契约 + 业务编排 + 数据访问 + agent 内核**。

```
2427 api_extensions     ← MCP/SKILL 的 HTTP + 校验 + 落盘（现最大，尚未拆）
1389 api_chat           ← 对话 HTTP（工具往返循环已搬 `quill-core/src/turn.rs`，Q012）
 956 api_experts        ← 专家 HTTP + 导入导出
 910 mcp_repo           ← MCP 配置的 SQL 口径（非 api_* 里最大）
 818 api_channels       ← 通道 HTTP
（已搬出 HTTP 层：tools 1402、mcp_client 1213、turn 661 在 quill-core；
  skillhub 族拆分后单文件最大 627；llm_providers 957 → 509 只剩持久化）
```

内核逻辑寄生在 HTTP crate = 内核**无法被单测**、**无法被 CLI 复用**、**无法换实现**。

### 1.6 冗余账（你担心的"冗余代码"）

| 冗余点 | 处数 | 位置 |
|---|---|---|
| 16 字节 id 转 hex | **≥6** | `api_chat.rs:311/315/321`、`api_teams.rs:452`、`llm_providers.rs:229`、`quill-backup::digest::to_hex`、`quill-control::secret::hex_encode`、`quill-adapters::ids` |
| hex 字符串转 id | **≥3** | `api_chat.rs:871/890`、`api_dispatch.rs:479`、`quill-control::secret::hex_decode_exact` |
| `now_ms()` | 6 处同名 | 各 api_* 各写一份 |
| 取值助手 `need_str`/`opt_str`/`s`/`n` | 5+5+… | 几乎每个 api_*.rs 复制一遍 |
| provider 组装 | **1 套（已修，2026-10-08 Q015）** | 组装只剩 `quill_core::llm::build`；`llm_providers.rs` 只做持久化 |

复现：

```bash
grep -rnE 'fn (hex|hex16|hex_lower|to_hex|parse_id|parse_public_id|parse_hex|hex_upper)' crates/*/src/
grep -rhoE '^(pub )?(async )?fn [a-z_]+' crates/*/src/**/*.rs | grep -oE 'fn [a-z_]+$' | sort | uniq -c | sort -rn | head
```

**SQL 越层**：`api_chat.rs` 内联 **22 条** SQL（`teams_repo` 24 条 / `experts_repo` 17 条是收口的，`api_chat` 却直接写）。

---

## 2. 结构性问题（按危害排序）

- **P-1 内核没有独立层（最重）**：按 `C1`，内核应**移植自 goose** 且独立成层；实际它散在 HTTP crate 里，且最重的两块（压缩/记忆）为零。**这是"地基不牢"的根**。
- **P-2 分层泄漏**：`quill-server` 既 HTTP 又编排又 SQL 又内核（内核部分 2026-10-08 起已按 Q013/Q014/Q015 搬出三块）；`quill-agent → quill-wiki` 的倒挂**已修**（`.layer-guard.mjs` 的基线里现在只剩 `quill-upgrade → quill-backup` 同层依赖）。
- **P-3 冗余**：id/hex 转换 6+ 处、取值助手 5+ 套、provider 组装 2 套、SQL 未收口。
- **P-4 名实不符**：`quill-agent` 不装 agent（仍在）；`quill-upgrade` 零调用 —— **2026-10-08 已不再成立**（`/api/upgrade/prepare` 真走它的升级前备份守卫）；**2026-10-09 起 `POST /api/upgrade/apply` 补齐了产物管线（下载 + 校验 + 暂存 + 交出命令）**，只剩「进程内自替换」这一件设计上做不到的事（Q063）。
- **P-5 空壳问题（正面案例）**：`quill-bridge`/`quill-ext-hub`/`quill-xtask` 已被移出（`Cargo.toml:21`）—— 说明"建了没人用"要被清掉。

---

## 3. 目标架构（To-Be）

### 3.1 分层规则（硬约束）

```
L5  quill-cli              命令行
L4  quill-server           ★只做 HTTP：路由 / 鉴权 / 序列化 / 状态码
L3  quill-core      (新)   ★内核层：对话循环、工具、MCP、provider 组装、
                              子 agent、上下文压缩、记忆
L2  quill-agent  quill-control  quill-wiki  quill-backup  quill-upgrade
L1  quill-domain
L0  quill-adapters  quill-store  quill-provider
```

> `quill-core` 里的每一块，**都标注它移植自 goose 的哪个文件**（见 §3.3），
> 便于日后与上游比对、跟着上游修 bug。

**依赖规则（违反即打回）**：

1. 依赖**只能向下**，不许反向（`quill-agent` 不许再依赖 `quill-wiki`）。
2. **L4 不许写 SQL**：一律经 L2 的 `*_repo`。id/hex 转换一律用 L0 单一出口。
3. **L0 不许依赖任何上层**。
4. 新建 crate 前先证明"现有 crate 放不下"；**不许再建没人依赖的空壳**（参照已删的 3 个）。
5. **不引入 goose/octop 作为依赖**（§0.1）。内核逻辑**抄进来**，标明出处。

### 3.2 内核层怎么落（抄，不依赖）

`quill-core` 的建立分两步，**第一步是纯搬运，零产品风险**：

- **第一步（搬运）**：把 `quill-server` 里寄生着的内核逻辑**原样搬进 `quill-core`** ——
  `api_chat` 的对话循环、`tools.rs`、`mcp_client.rs`、provider 组装。行为一个字不改、
  测试一个不减。这步做完，内核就能被单测、能被打包、能被 CLI 复用。
- **第二步（补课）**：按 goose 的参考源，**逐块补齐缺失的内核能力** ——
  上下文压缩（`goose-context-management`）、记忆、状态机/快照/重试（`goose/agents/`）。
  每补一块，标注出处并补测试。

**为什么这样切**：你担心的"冗余/倒塌"根因是**内核没有层**。先建层、再补内容，
比"边加功能边想分层"稳得多；而"照 goose 补课"天然对齐 `C1`，不会各写一套。

### 3.3 模块归属表（防冗余的"唯一出口"）

| 能力 | 唯一归属 | 参考源 | 禁止 |
|---|---|---|---|
| id ↔ hex | `quill-adapters::ids` | — | 任何 crate 再写 hex 转换 |
| 取值助手 | `quill-server::jsonx`（新） | — | 各 api_*.rs 复制 |
| `now_ms` | `quill-store` | — | 6 处各写 |
| SQL | 各 L2 `*_repo` | — | L4 内联 |
| provider 组装 | `quill-core`（用 `quill-provider`） | `goose-providers` | `llm_providers.rs` 再拼一套 |
| 对话循环 | `quill-core` | `goose/agents/agent.rs` | L4 内联 |
| 工具执行 | `quill-core` | `goose/agents/tool_execution.rs` | L4 内联 |
| MCP | `quill-core` | `goose/agents/mcp_client.rs`、`goose-mcp` | L4 内联 |
| 上下文压缩 | `quill-core` | **`goose-context-management`** | 自创算法 |
| 记忆 | `quill-core` | `goose/session` 相关 | 自创 |

---

## 4. 需要你拍板的架构决策

> 我只在**真正改产品契约**时才停下来问。其余照做。

**D1 内核层先建哪一块？**（§0.1 已定"抄不依赖"，剩下的是**顺序**）

- **D1-a（推荐）**：先**搬运**（把 `api_chat`/`tools`/`mcp_client` 原样搬进 `quill-core`），
  再按 goose 补上下文压缩与记忆。纯重构起步，风险最低。
- **D1-b**：先补**最缺的** —— 直接照 `goose-context-management` 实现上下文压缩，
  内核层边补边成形。见效快，但会先欠一层结构债。

我建议 **D1-a**。**你没空回我就按 D1-a 走**，因为它是纯重构、不改对外行为。

**D2** `quill-agent` 正名为 `quill-team`（或并入 `quill-domain` 的应用子模块），
把"agent"这个名字让给内核层。纯改名。

**D3** `quill-upgrade` 要么被 `quill-cli`/`quill-server` 真正调用，要么删（像已删的 3 个空壳）。

---

## 5. 优先级路线图（重要节点 × 易实现）

### P0 地基修复（低风险、高收益，**不依赖任何决策，现在就能做**）

| # | 项 | 为什么重要 | 为什么容易 |
|---|---|---|---|
| P0-1 | id/hex 转换收口到 `quill-adapters` | 消灭 6+ 处重复，消除"某处行为漂移"隐患 | 纯搬函数 + 替换调用点 |
| P0-2 | `api_chat` 的内联 SQL 收口到 repo | 解耦巨石最重的一块 | 逐条搬，行为不变 |
| P0-3 | 取值助手统一（`need_str`/`opt_str`/`s`/`n`） | 5+ 套复制是纯浪费 | 建小模块，全局替换 |
| P0-4 | 去掉 `quill-agent → quill-wiki` 生产依赖 | 修层次倒挂 | 看它用了 wiki 什么，再决定搬走或注入 |

### P1 重要节点（内核层）

| # | 项 | 说明 |
|---|---|---|
| P1-1 | 建 `quill-core` 并搬运内核逻辑（D1-a 第一步） | 纯重构：对话循环 / 工具 / MCP / provider 组装 |
| P1-2 | 照 `goose-context-management` 实现上下文压缩 | 当前只有配置字段、零实现 |
| P1-3 | 记忆 | 迁移清单里最重的一块 |
| P1-4 | 内核状态机 / 快照 / 重试（照 `goose/agents/`） | 长程任务不中断的底子 |
| P1-5 | 逐块比对 provider 移植覆盖度，合并两套组装 | 消除 §1.6 的 provider 冗余 |

### P2 易实现的小功能（不阻塞，随时插空）

来源：`docs/OCTOP-MIGRATION-INVENTORY.md` 里「底层已实现、只差接线」的项。
**本轮示例**：`/api/extensions/bundle/export|import`（需求 4 多端同步，`api_bundle.rs`）。

---

## 附：本文档所有事实的复现命令

```bash
# 依赖图
for d in crates/*/; do n=$(basename "$d"); \
  deps=$(sed -n '/^[[]dependencies[]]/,/^[[]/p' "$d/Cargo.toml" | grep -oE 'quill-[a-z]+' | grep -v "^$n$" | sort -u | paste -sd' ' -); \
  echo "$n -> $deps"; done

# goose/octop 不是依赖（符合意图）
grep -rnE '^\s*(goose|octop)' crates/*/Cargo.toml Cargo.toml          # 空

# goose 内核参考源规模
for d in vendor/goose/crates/*/; do n=$(basename "$d"); \
  printf "%-24s %6s\n" "$n" "$(cat "$d"/src/*.rs 2>/dev/null | wc -l)"; done
ls vendor/goose/crates/goose/src/agents/

# 内核缺位：压缩/记忆
grep -rniE 'compaction|context.management|记忆|memory' crates/*/src/ | grep -v test

# 巨石
for f in crates/quill-server/src/*.rs; do printf "%6d %s\n" "$(wc -l <"$f")" "$(basename $f)"; done | sort -rn | head

# 冗余
grep -rnE 'fn (hex|hex16|hex_lower|to_hex|parse_id|parse_public_id|parse_hex|hex_upper)' crates/*/src/
```
