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

### B2. 通道（channels）：部分能力只做诚实降级

**上游**：octop 的通道（微信等）有完整收发链路。

**quill**：`quill-server/src/channels/` 有实现，但没做到的地方一律**如实标注**
（前端 `capabilityGaps.ts` 登记「已登记路由、处理函数未实现」），不画假按钮。
逐条对齐仍缺：queue Q045。

**为什么**：这是「先有真后端再放按钮」的纪律（最高指示第 2/5 条），
宁可页面上少一个开关，也不给一个点了必失败的东西。

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

# B1：派工记账会话（建它的是 teams_repo，不是 api_teams —— 这条命令原来是错的，已改）
grep -c "team_leader" crates/quill-server/src/api_chat.rs crates/quill-server/src/teams_repo.rs

# C1/C2：两道自造门禁
node .layer-guard.mjs --self-test
node .upstream-check.mjs
```
