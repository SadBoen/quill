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

## 3. 端口二：`ProviderAssembly`（给 Q015 / provider 组装）

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

---

## 5. 搬运的通用配方（Q012/Q013/Q015 共用）

1. 内核侧先定义端口与行类型（本文档 §2–§4 的形状），**只加不改**；
2. 把文件 `git mv` 进 `quill-core`，把 `AppState`/`DbBridge`/repo 调用换成端口调用；
3. 壳侧写 impl（转发到既有函数，**行为不改**），`quill-server` 里保留同名 re-export，
   让既有调用点零改动；
4. 内核侧补假实现测试；真库覆盖搬到壳侧集成测试；
5. 门禁：`cargo test --workspace` **总数不减**、`clippy -D warnings` 退出 0、`fmt --check` 退出 0；
6. 提交信息写清「哪些调用点从壳改成了端口」「哪些测试搬到了哪一侧」。
