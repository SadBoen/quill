# 内核端口设计（`quill-core` 与壳的边界）

> 2026-10-08 立。**这份文档回答一个问题**：`quill-core`（L3 内核层）要搬进
> `tools.rs` / provider 组装 / 对话循环，但这些代码现在直接调用
> `AppState`、`DbBridge` 与各 repo —— 而内核**不能依赖** `quill-server`（层次倒挂）。
> 边界怎么划、拿什么替换，就是本文档的内容。
>
> 复现本文档依据的命令在每节末尾。凡与代码不符的，以代码为准并回来改本文档。

---

## 0. 一句话

**内核只声明「我需要什么」（trait），壳负责「从哪拿」（impl）。**
内核侧的类型（行数据）住在内核里；壳侧的实现（SQL、文件路径、探测）住在壳里。

这不是自创：goose 的做法就是把 agent 逻辑与扩展/会话存储分开，
`vendor/goose/crates/goose/src/agents/` 里的 `ExtensionManager` 等就是「内核要什么」
的接口面，具体扩展由外部注册。quill 的差别只在：我们用 Rust trait 显式化。

---

## 1. 已经立住的样子（Q014 的示范）

`mcp_client.rs` 能搬进内核，是因为它只需要一个**数据结构**（`McpServerRow`）与 rmcp：

- `McpServerRow` 定义搬进 `quill-core::mcp`（内核自己的模型）；
- `quill-server::mcp_repo` 改成 `pub use quill_core::mcp::McpServerRow;`（路径不变）；
- 存储（SQL）留在壳里，内核只拿「已经查好的行」。

**这条模式对「纯数据」够用；对「需要查询/落盘的动作」不够用** —— 那就是端口。

---

## 2. 端口一：`ToolSources`（给 Q013 / `tools.rs`）

内核要的是**三份已经查好的清单**，不是数据库句柄：

```rust
/// 内核要的「工具素材从哪来」。壳侧实现（SQL + 技能正文落盘位置）在
/// `quill-server::tool_sources`。
pub trait ToolSources: Send + Sync {
    fn experts_for_tools(&self, uid: UserId) -> Result<Vec<ExpertToolRow>, ToolSourceError>;
    fn skills(&self, uid: UserId) -> Result<Vec<SkillToolRow>, ToolSourceError>;
    fn mcp_servers(&self, uid: UserId) -> Result<Vec<McpServerRow>, ToolSourceError>;
}

pub struct ExpertToolRow { pub id: String, pub display_name: String, pub description: String, pub instructions: Option<String> }
/// `body` 是技能正文**已读好的文本** —— 内核不该知道技能文件放在哪（那是壳的路径口径）。
pub struct SkillToolRow { pub name: String, pub description: String, pub enabled: bool, pub tool_allowlist: Vec<String>, pub body: String }
pub struct ToolSourceError(pub String);   // 只带给人看的中文消息，内核不解释它
```

约束：

- `ToolRegistry::builtin*` / `with_mcp_tools` 收 `Arc<dyn ToolSources>` 而不是 `Arc<AppState>`；
  MCP 的**探测与调用**（`discover_all` / `call_tool_blocking`）本来就在内核里（Q014 已搬），不动。
- `digest32` 是**纯函数**，跟着 `mcp_tool_name` 搬进内核（`quill_core::digest`），
  `quill-server::db::digest32` 改成 re-export —— 否则内核要为了一个哈希函数依赖壳。
- 壳侧 impl 转发到既有函数（`api_experts::list_for_tools` / `skills_repo::list` +
  `api_extensions::{skill_body_path, read_skill_body}` / `mcp_repo::list`），**行为一个字不改**。
- **测试**：内核侧用假 `ToolSources`（内存里的 Vec）测注册表逻辑；
  真库覆盖留在壳侧（集成测试用真 impl 建注册表，断言工具表）——
  「测试数量不减」按 **workspace 总数**算，包级数字会随搬家变化，报告里要写清。

复现（当前耦合面）：

```bash
grep -rn "crate::state::AppState\|crate::db::\|crate::api_experts::\|crate::skills_repo::\|crate::mcp_repo::\|crate::api_extensions::" crates/quill-server/src/tools.rs
grep -rn "crate::tools::" crates/quill-server/src/ | grep -v "^crates/quill-server/src/tools.rs"
```

---

## 3. 端口二：provider 组装（Q015 **已落地 2026-10-08**）

> **落地结果**：本节的边界已按下面执行完毕 —— `llm.rs`（配置 + `build()`）整体搬进
> `quill-core/src/llm.rs`；探测与解析（`ModelCard` / `parse_models_payload` /
> `probe_models` / 模型行模型 `LlmProvider`）搬进 `quill-core/src/providers.rs`；
> 壳侧 `llm_providers.rs` 只剩 9 条 SQL 的持久化（957 → 509 行）并对内核做 re-export。
> **复现「只剩一套」**：`grep -rn "OpenAiCompatible::new" crates/ --include=*.rs | grep -v quill-provider`
> → 生产侧只有 `crates/quill-core/src/llm.rs:206` 一处（其余是 quill-provider 自己的测试）。
> 未做（如实记）：`quill-provider::Provider::models()` 与内核探测仍是**两个契约**
> —— 前者只回 `{id, owned_by}`，后者要上游原始字段来推 `context_window`/`modality`；
> 合并不是把两者并成一个函数，见 `docs/KERNEL-ALIGNMENT.md §Q029` 的差集表。

现状（`docs/KERNEL-ALIGNMENT.md §Q029` 已核实）：**两套** —— `quill-provider`（传输层，
3241 行，不碰 DB）与 `quill-server::llm_providers` + `llm.rs`（DB 配置 / 探测 / 组装，
1345 行，9 条内联 SQL）。真正重复只有 3 处：`/models` 探测、base_url 归一、传输错误分类。

边界（按「谁需要什么」切，不按文件切）：

- **内核**：`LlmConfig` → `SharedProvider` 的**构造**（协议选择、base_url 归一、
  请求构造），以及 `/models` 探测的**解析**。产物是一个 provider 句柄，内核拿它跑对话循环。
- **壳**：`llm_providers` 表的增删改查（SQL 留在壳，按 L4 不写 SQL 的规矩收在 repo）、
  `AppState` 里的 `llm_config` 装配、以及把 DB 行转成内核要的 `LlmConfig`。
- 合并口径：**传输实现只有 `quill-provider` 一份**；壳不再自己拼 HTTP 客户端，
  只把配置交给内核的构造函数。重复的 3 处各自留一份（在 `quill-provider` 侧），
  另一份删除并在提交信息里点名。

复现：

```bash
grep -c "sqlx::query" crates/quill-server/src/llm_providers.rs
grep -rn "fn models_url\|probe_models" crates/ --include=*.rs | head
```

---

## 4. 端口三：对话循环要的四类端口（给 Q012）

对话循环（`api_chat::run_turn`）与 HTTP 无关的部分是**「一轮怎么推进」**：
取历史 → 建请求 → 流式取回 → 有工具就执行并回灌 → 直到正文或预算耗尽。
它需要四类东西，全部走端口：

| 端口 | 内核要什么 | 壳侧现在是谁 |
|---|---|---|
| `TurnHistory` | 读最近历史（角色+正文） | `chat_repo::history_rows` |
| `TurnRecorder` | 记一条消息（含用量）、推进会话计数 | `chat_repo::{insert_message, touch_session}` |
| `ProviderFactory` | 按会话配置拿到 provider 句柄 | `state.llm` / `providers` |
| `ToolSources` | 同 §2 | `tool_sources`（Q013 落地） |

**HTTP / SSE / 鉴权 / 限流 / 错误信封不进内核** —— 那是壳的活。
内核产出「事件」（正文增量、工具往返、结束原因），壳负责把它编码成 SSE 或 JSON。

### 4.1 Q012 现状与搬运清单（2026-10-08 清点，2026-10-09 **已搬运**）

行号是清点时的（`crates/quill-server/src/api_chat.rs` 共 1648 行、`api_chat_stream.rs` 222 行）：

| 现在的样子 | 位置 | 搬/留 | 怎么改 |
|---|---|---|---|
| `TurnPrep`（db / uid / sid / provider / llm_config / registry / tools / user_created_at） | `api_chat.rs:827` | **拆** | 内核要的那半（provider / llm_config / tools / registry）进 `quill_core::turn::TurnInput`；壳那半（db / uid / sid / 消息 id）留在壳 |
| `ReplyMode`（`Once` / `Streamed`） | `:994` | 搬 | 内核只认「要不要流」这一位，与 HTTP 无关 |
| 取回循环本体（建请求 → 流式或一次性 → 有工具就执行回灌 → 再问） | `:1073-1190`（`run_turn`） | **搬** | 进 `quill_core::turn::run_turn`，产出 `TurnOutcome { text, reasoning, usage, turn_ms, rounds, exhausted }` |
| 工具预算闸 `MAX_TOOL_ROUNDS` | `:1094`、`:1160` | 搬 | 跟着循环走（该常量已在 `quill_core::tools`） |
| 工具执行 `prep.registry.call(call)` | `:1109` | 搬 | `ToolRegistry` 已在内核（Q013），直接调 |
| 增量转发 `StreamDelta` → sink | `:1035-1045`；SSE 侧 `api_chat_stream.rs:68` | **搬成观察者** | 内核定义 `TurnObserver`（`on_text` / `on_reasoning` / `on_tool_round`）；SSE 壳实现它写事件，非流式壳给空实现 |
| 用户消息落库 `append_message` | `:870`、`:1432` | 留壳 | 内核不写库；壳在调 `run_turn` **之前**写用户消息（现在也在这之前） |
| 助手消息落库 + `touch_session` | `:1259`、`:1276`、`:1472` | 留壳 | 内核返回 `TurnOutcome` 后由壳写。**先不引入 `TurnRecorder` 端口** —— 返回值就够，等真需要（例如内核要中途落库）再加，别为对称而对称 |
| `user_message_json` / HTTP 错误映射 | `:1195`、`provider_failure` | 留壳 | 纯 HTTP 契约 |

**搬运顺序（每步都能单独跑绿）**：
1. 内核加 `turn.rs`：`TurnInput` / `TurnOutcome` / `TurnObserver` + 循环本体（从 `run_turn` 复制过来，**先不改调用方**）；
2. 内核补测试：假 provider（照 `quill-core/src/tools.rs` 里 `Recorder` 桩的写法）+ 假 registry，覆盖「无工具一轮」「工具往返后收正文」「预算耗尽」「流式观察者真收到增量」；
3. 壳侧 `run_turn` 改成薄壳：拼 `TurnInput` → 调内核 → 落库 + 拼响应/SSE；
4. 删掉壳侧循环残留，跑 `cargo test --workspace`（总数不减）与 `.layer-guard.mjs`；
5. 提交信息写清「哪些行从壳搬进了内核」「SSE 路径怎么变成观察者」。

**判据**：`grep -c "MAX_TOOL_ROUNDS\|registry.call" crates/quill-server/src/api_chat.rs` → 0；
`cargo test --workspace` 总数不减；`api_chat.rs` 行数明显下降（1648 → 预计 ~1300）。

### 4.2 Q012 落地结果（2026-10-09）

计划与实做的**差异**（照实记）：

- 新增 `crates/quill-core/src/turn.rs`（661 行，含 7 条测试）：`TurnInput` / `TurnOutcome`
  / `TurnUsage` / `ReplyMode` / `TurnObserver` 与循环本体全在这里。
- **`TurnInput` 用借用而不是所有权**：`provider` / `llm_config` / `registry` / `tools`
  都是 `&`，只有 `messages` 取走（循环要往里追加工具往返）。计划里写的「壳那半留在壳」
  照做 —— `TurnPrep` 一个字没改结构，只是它不再有循环。
- **`TurnObserver` 的方法名与计划里的素描不同**：最终用 `round_start` / `text` /
  `reasoning` / `tool_call` / `tool_result` / `discard` 六个（计划里的
  `on_text` / `on_reasoning` / `on_tool_round` 是三合一的简写）。它就是壳里原来的
  `RoundSink` —— **改名搬进内核**，方法签名逐字不变，`api_chat_stream.rs` 的
  `impl` 只换了 trait 名。`NullSink` 留在壳（「什么都不做」是一种用法，不必进内核）。
- **错误映射**：内核返回 `quill_provider::ProviderError`，壳在薄 wrapper 里
  `map_err(provider_failure)`。内核不认识 `ApiError`，也不该认识。
- **`TurnUsage` 的累计规则文档整块搬进内核**（那是这个类型自己的规矩）；壳的测试
  改成 `use quill_core::turn::TurnUsage`。
- 新增一个内核侧小件：`impl Default for ToolRegistry`（空表是合法状态）。内核测试
  用它起步再 `register` 桩；没有别的构造路径可走（字段私有，同 crate 的兄弟模块
  也读不到）。

**判据复现（2026-10-09 实测）**：

```bash
grep -c "MAX_TOOL_ROUNDS\|registry.call" crates/quill-server/src/api_chat.rs   # → 0
wc -l crates/quill-server/src/api_chat.rs                                      # → 1389（原 1648）
cargo test --workspace                                                          # → 1459 passed / 0 failed（原 1452，+7 = 内核 turn 测试）
```

`quill-server/src` 合计 28554 行（原 28692）：净减因为循环搬走了，`quill-core`
相应增加。

---

## 5. 搬运的通用配方（Q012/Q013/Q015 共用）

1. 内核侧先定义端口与行类型（本文档 §2–§4 的形状），**只加不改**；
2. 把文件 `git mv` 进 `quill-core`，把 `AppState`/`DbBridge`/repo 调用换成端口调用；
3. 壳侧写 impl（转发到既有函数，**行为不改**），`quill-server` 里保留同名 re-export，
   让既有调用点零改动；
4. 内核侧补假实现测试；真库覆盖搬到壳侧集成测试；
5. 门禁：`cargo test --workspace` **总数不减**、`clippy -D warnings` 退出 0、`fmt --check` 退出 0；
6. 提交信息写清「哪些调用点从壳改成了端口」「哪些测试搬到了哪一侧」。

---

## 6. 记忆（Q019，2026-10-09 落地）

**它不是「端口」这一类搬运** —— 记忆是 goose 的**一个内置 MCP 扩展**，不是从
`quill-server` 搬过来的现有逻辑。所以这一节记的是**从哪里抄、抄成什么样**。

### 6.1 上游长什么样

`vendor/goose/crates/goose-mcp/src/memory/mod.rs`（851 行，goose v1.53.0）：
`MemoryServer` 是一个 rmcp 服务器，四个工具
（`remember_memory` / `retrieve_memories` / `remove_memory_category` /
`remove_specific_memory`），背后是**按分类落盘的 `.txt`**（一分类一文件；
一次 `remember` 追加「可选 `# 标签` 行 + 正文 + 空行」；一次 `retrieve` 按空行切条）。
goose 把它登记进 `goose-mcp/src/lib.rs` 的 `BUILTIN_EXTENSIONS`，有两条入口：
**进程内**（`goose/src/agents/extension_manager/builtin.rs:19-33` 的
`tokio::io::duplex`）与 **stdio**（`goose mcp memory`，
`goose-mcp/src/mcp_server_runner.rs:36-49`）。

### 6.2 quill 抄成什么样

- **落点**：`crates/quill-core/src/memory.rs`（存储语义 + 四个工具 + 分类名安全边界 +
  全局记忆拼进 instructions）。
- **stdio 入口**：`quill mcp memory`（`crates/quill-cli/src/main.rs` 的 `run_mcp`），
  对应上游的 `goose mcp memory`。**只走 stdio**：quill 的 `mcp_client` 只铺了 stdio，
  进程内 duplex 那条没有对应实现（记进 `docs/UPSTREAM-DIVERGENCES.md` A6）。
- **为什么在 `quill-core`**：`docs/ARCHITECTURE.md §3.3` 的映射表里「MCP」本来就
  归 `quill-core`（上游列的是 `goose/agents/mcp_client.rs`、`goose-mcp`）。
- **依赖形状**：为它把 rmcp 的 `server` / `macros` / `transport-io` 三个 feature 打开，
  并直接依赖 `serde` / `schemars`（上游 `goose-mcp` 的 `Cargo.toml` 里两者都在）。
  **都不是新增第三方依赖**，是把 rmcp 自己的可选模块打开。

### 6.3 还没做（如实记）

- **记忆还没接进任何一轮对话**：没有任何地方把 `quill mcp memory` 登记成一台 MCP
  服务器，模型现在还调不到它。与 Q017/Q020/Q022 同一状态（内核逻辑先落地、接线另记）。

### 6.4 判据复现（2026-10-09 实测）

```bash
cargo test -p quill-core --lib memory          # → 13 passed / 0 failed
cargo test -p quill-cli --test mcp_memory_stdio # → 1 passed / 0 failed（真子进程 + 手写 JSON-RPC）
```
