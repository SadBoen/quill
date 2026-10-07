# BACKLOG —— 还差什么，按里程碑分组

> **重建说明**：本文件曾由上一批人维护，内容不可信。现以代码为准重写。
> 本文件**只讲为什么**（背景、依赖）；**状态跑 `node scripts/status.mjs` 看**，不写在这里。
> 判据声明在 [`project/items.mjs`](project/items.mjs)。事实总账见 [`docs/CODE-TRUTH.md`](docs/CODE-TRUTH.md)。

---

## 状态：跑命令看，不要读这里

```bash
node scripts/status.mjs          # 全量
node scripts/status.mjs --quick  # 秒级
node scripts/status.mjs --json   # 机器可读
```

三种结果的含义不许混：

| 结果 | 含义 |
|---|---|
| **已验证** | 判据跑过且通过 |
| **未通过** | 判据跑了但没过 |
| **需人工** | **没有被机器检查过** —— 既不是通过也不是失败，是公开的缺口 |
| **判据本身坏了** | 绑的名字/种类对不上 —— 真实状态**未知**，比「未通过」更严重 |

---

## M0 · 可重复的地基

### B0-1 门禁判定逻辑本身要被测试
防止「恒为真的门禁」。现例：`.scripts/gate-selftest.sh` 与 `gates.sh` 的解析逐字同源。

### B0-2 一条命令跑完全部门禁
`bash .scripts/gates.sh` 退出 0 才算活。覆盖：构建 / Rust 全部测试 / 前端
typecheck·lint·vitest·build / 四道文本门禁。

### B0-3 `cargo fmt` / `cargo clippy` 从未进门禁
`gates.yml:51` 原话「刻意不开 clippy/rustfmt」。实测：`cargo clippy` 退出 0（仅 warning）、
`cargo fmt --check` 有 **481 个文件**差异。**这是有意的空白**，不是回归。
要不要采用 formatting 是**产品决定**，不该顺手做掉。

---

## M1 · 单人闭环

### B1-1 界面上不存在「点了必失败」的按钮
判据只能人工：失败只在真点下去之后才存在。每个入口要么真通，要么明确写成未接通。

### B1-2 会话改名返 405，界面要如实提示
`PATCH /api/sessions/{id}` 未注册（`routes.rs` 只挂 get + delete）。界面按实情提示。

### B1-3 未注册 ≠ 501，界面要分清三种
`capabilityGaps.ts` 要区分：已实现 / 已登记但 501 / 路由压根不存在（404）。

### B1-4 首管之后浏览器实点验收
判据只能人工。逐页点一遍并贴实际结果。

---

## M2 · 专家与专家团真执行（**当前最大的功能缺口**）

### B2-1 派工只记账，没有消费者
`api_dispatch.rs` 的 `POST /api/teams/{id}/dispatch` 只写台账。
**台账键是 `(owner, room_id, round)`，不是 `team_id`** —— 要真正按团队过滤得改键，
那是产品契约变更，先问。

### B2-2 成员执行器不存在
以代码为准：生产侧唯一的 `impl MemberExecutor` 是 `quill-agent/src/dispatch.rs:739`
的泛型 `SharedExecutor<E>` 包装；**没有任何具体实现**，
`MockMemberExecutor` 只在 `quill-testkit`（测试用）。
参照 `vendor/goose/crates/goose/src/agents/subagent_handler.rs` 的 `run_subagent_task`
（每子 agent 独立 config + 独立 session）来做，并注明哪些自研。

### B2-3 建团队静默多出的 `team_leader` 会话
`teams.leader_session_id` NOT NULL + 外键 → `POST /api/teams` 会插一条
`kind='team_leader'` 的会话。产品决定：侧栏隐藏（`GET /api/sessions?exclude_kind=`）。

### B2-4 teams 的限制列是摆设
`teams.guidelines` / `max_dispatch` / `max_replan` / `max_ask_depth` 在生产 Rust 里**零引用**。
真要做 M2 派工限额时才有消费者。

---

## M3 · 上下文与成本可信

### B3-1 对外报了一个不存在的开关
`compaction_threshold_tokens` 只出现在配置 / 校验 / 落库 / 上报里，**没有运行时读者**。
`sessions` 有 8 列（`summary` / `summary_upto_seq` / `compacted_count` / `checkpoint_key` /
`checkpoint_path` / `replan_count` / `error_state` / `last_error`）在 server 侧零读写。
压缩本体参照 `vendor/goose/crates/goose-context-management`。

### B3-2 token 口径
goose 的 `input_tokens` 已含 cache，缓存是子集，总量按减法算。跨轮累加、`cache_read`
从不折进 `input`。

### B3-3 上下文图加载态什么都不渲染
`ContextWindowChart` 加载中直接 `return null`，没有骨架、没有 `aria-busy`。

---

## M4 · 数据安全

### B4-1 备份能导出、能校验、能真还原
`quill-backup` 有 `create_backup` / `restore_backup` / manifest / sha256。
`POST /api/backup/restore` **不在进程内还原**（服务占着数据库文件），返回 CLI 命令。

### B4-2 在线升级
`crates/quill-upgrade/src/lib.rs` 只有 **120 行**，内容是 `PreUpgradeGuard` +
`take_pre_upgrade_backup`。`quill-cli` 声明了依赖却**零调用**。
真正的在线更新机制**不存在**。

---

## M5 · 多用户与多端同步

### B5-1 用户管理三层
`GET /api/users`（真读库 + 分页）、`PATCH /api/users/{id}`（真启停）已通；
`POST` / `DELETE` **有意保持 501**（账号只来自部署配置）。

### B5-2 `/api/cron` 连路由都没注册 → 404
`ui/web/src/automations/api.ts` 配的是假后端。

### B5-3 三张表只存在于迁移与测试
`plugins` / `wiki_index` / `skills.tool_allowlist` 与 `expert.skill_count`（恒写 0）
在生产侧没有真消费者。**上报没有真来源的字段 = 骗人**，先摘掉上报、等真做了再挂回去。

---

## M6 · 与两个上游对齐

### B6-1 版本基线
`node .upstream-check.mjs`（离线核本地）；`--online` 核上游是否前进（落后退出非 0）。

### B6-2 自创机制要标出来
`octop` 的界面不用 ECharts（它用 recharts），所以 quill 的 ECharts 封装是**自创**，
引用时必须这么写。

### B6-3 上游引用要有出处
`node .provenance-check.mjs` 逐条核 `UPSTREAM-USAGE.md` 的引用。
**注意**：它核的是那份文档的记录，文档本身也要以代码复核。

### B6-4 `vendor/openoctopus-frontend` 不是 Octop
它是 `Zpoieiti/OpenOctopus`，只贡献 `index.css`。`.octop-baseline-check.mjs` 拦「把它当 Octop 用」。

### B6-5 通道与个性化页
已迁：迁移 `0009_channels.sql`、`channels/{store,weixin}.rs`、`api_channels.rs`、
`ui/web/src/channels/`。仍缺：**没做过微信真机扫码**；MBTI 已在 B6-6 落地。

### B6-6 人格（MBTI）
已迁：`mbti/{profiles,questions,score,store,apply}.rs`、`api_mbti.rs`、`0010_mbti.sql`、
`ui/web/src/mbti/`。三处「我们的选择」：不写 SOUL.md、`apply` 强制带 `expert_id`、存历史。

---

## 代码里查出的真实缺陷（与功能无关，但要修）

1. `quill-cli` 声明 `quill-upgrade` 依赖却零调用。
2. `quill-server` 是 26616 行巨石。
3. `teams` 四个限制列零引用（见 B2-4）。
4. `wiki_index` / `skill_count` / `tool_allowlist` 仍在上报但无真来源（见 B5-3）。
