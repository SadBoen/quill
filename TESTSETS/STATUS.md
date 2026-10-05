# 测试集进度表

100 条真实任务（50 MCP-Atlas + 50 SkillsBench），来源与许可见 `README.md`。
生成时间：2026-10-06。

## 前置条件（不满足就别开始跑）

- [x] SKILL 已通过 `skills_repo::as_tool_spec` 挂进 `ToolRegistry`（对话里能真的调）
      —— 2026-10-06 完成。`ToolRegistry::with_skills`（async，行在库里、正文在磁盘上），
      `api_chat.rs` 构造完 registry 就调。只挂 `enabled` 的；磁盘没正文的跳过而不是
      注册空工具；与已有工具同名的跳过而不是顶掉它；查库失败**整条请求失败**，
      不静默降级成「只有内置工具」。用户过滤复用 `skills_repo::list` 的 `user_id`。
- [x] 界面如实显示「模型看不看得见」—— 2026-10-06 完成。`GET /api/extensions/skills`
      每条带 `model_can_see`，挂不上时带 `not_mounted_reason`。判断走
      `tools::skill_visibility`，与 `with_skills` **同一个函数**（两处各判一次必然漂）。
      界面文案里「技能包尚未挂进对话的工具表」这句已过时的假话已改掉。见 ISSUE-009。
- [x] MCP 协议层已接（`rmcp`），`GET /api/extensions/mcp` 的 `connected` 不再恒为 false
      —— 2026-10-06 完成。`mcp_client::probe`（`crates/quill-server/src/mcp_client.rs`）
      真的用 `TokioChildProcess` 拉起 stdio 子进程、真的 `initialize`、真的 `tools/list`
      （含游标翻页，页数有上限 8）。`GET` 与 `POST` 共用 `mcp_body`，**保存也重测一次** ——
      前端保存后用 POST 响应替换列表缓存，那里回一个恒 false 等于刚存好就骗界面。
      每台服务器在 `status` 数组里带 `probed` / `connected` / `tool_count` / `error` /
      `protocol_version` / `server_info`，**只读状态与 `servers` 分开**（前端会把
      `servers` 原样回填进编辑表单再 POST，混进只读字段会被字段白名单拒掉）。
      界面按 `probed` 把「没查过」与「查了没通」分开显示。
      端到端测试的对端是**手写的** stdio MCP 服务器 `src/bin/mcp_stub.rs`，
      刻意不用 rmcp 起 server —— 两边共用同一份类型的话，协议层写错会两边一起错。
      `streamable_http` / `sse` 明确报「还没铺」，不假装连过。见 ISSUE-011、ISSUE-012。
- [x] **MCP 工具已挂进对话的工具表** —— 2026-10-06 完成。
      `ToolRegistry::with_mcp_tools`（async，紧挨 `with_skills`），`api_chat.rs` 构造完
      registry 就调。上一版列的三件事都定了：
      1. **挂上去叫什么**：`{服务器}__{工具}`，中间**两个**下划线。服务器名被
         `normalize_name` 归一成 `[a-z0-9-]`、永不含下划线，所以 `__` 既是合法分隔符
         又是唯一的 —— 挂载名与 SKILL 同名在结构上不可能（`skills.name` 的 CHECK
         连下划线都不许）。工具侧字符收窄到 `[A-Za-z0-9_-]`，超 64 字符截断并补
         8 位摘要（光截断会让两个长名撞成一个，然后被「同名顶掉」静默跳过）。
         **原名一起留着**：描述里带原名，`GET /api/extensions/mcp` 也报
         `mounted_tools: [[挂载名, 原名], …]`。
      2. **同名怎么办**：`tools::mcp_tool_visibility`，与 `skill_visibility` 同一个
         形状、同一个理由。撞名**跳过并报原因**，绝不 `register` 顶掉。
         撞名在真实数据上可达：`read--note` 与 `read-note` 归一后同名。
      3. **执行体是同步的**：`mcp_client::call_tool_blocking`。另起一条线程与一个
         current-thread 运行时，**不**用 `Handle::block_on`（在 async 上下文里 panic，
         `#[tokio::test]` 的 current-thread 也会炸）。代价是调用方那个 worker 线程
         会阻塞等结果，由 `MAX_TOOL_ROUNDS` 与 `CALL_BUDGET_CEILING_MS`（60s）封顶。
         `max_concurrent_calls` 真的生效了：按 `(用户, 服务器名)` 记的 `Semaphore`。
- [x] **`tools/call` 真的通了** —— `mcp_client::call_tool`。`initialize` → `tools/call`
      → 收正文，每次调用重新拉起子进程（探测那次的进程在返回前就收了）。
      `isError: true` 走 `Err`（让模型自己纠正）；`structuredContent` 在没有文本时顶上；
      非文本块**明说**不悄悄吞；MRTR / task / 未知结果类型各报各的白话。
- [x] **界面如实报「模型调得到」** —— `status` 里每台带 `mounted` / `mounted_tools` /
      `not_mounted`，顶层带 `mounted_count`。判断走 `tools::mcp_tool_visibility`，
      基线工具表走 `tools::baseline_specs`（与 `with_skills` 同一条链），
      **与 `with_mcp_tools` 同一个函数、同一次握手**。
      「连上了」与「挂上了」是两条独立事实，各自有数、各自有原因。
- [x] 全量门禁 0 failed —— 2026-10-06：`cargo test --workspace` 57 个目标
      957 passed / 0 failed、`ui/web` 68 passed、`typecheck` 干净、
      `i18n-check` 0 问题、`library-check` 334/334。
      **注意**：`.wsl-verify-persona.sh` **只跑 Rust**（`cargo build` + `cargo test`），
      不碰前端。前端那四项要在 `ui/web` 目录下另外跑：
      `npm run typecheck`、`npx vitest run`、`node ../../.i18n-check.mjs`、
      `node ../../.library-check.mjs`。**WSL 里没有 node**，只能在 Windows 侧跑。

## 进度

| 段 | 条数 | 已跑 | 通过 | 失败 | 待跑 |
|---|---|---|---|---|---|
| MCP-Atlas | 50 | 0 | 0 | 0 | 50 |
| SkillsBench | 50 | 0 | 0 | 0 | 50 |
| **合计** | **100** | **0** | **0** | **0** | **100** |

## 判定口径

每条任务记三件事，**不要混为一谈**：

1. **链路是否通** —— 请求有没有正常往返、界面有没有如实反映状态。
   这条不过就是 quill 的 bug，必须记进 `ISSUES.md`。
2. **工具是否被调用** —— 该调的工具有没有出现在 `tool_calls` 轨迹里。
3. **答案是否对得上** —— 用 `expected_claims` 粗判。本机是 4B 小模型，
   答不上来**不算 bug**；但「调了工具却把正文吞了」「明明没调工具却
   声称查过了」要算。

## 已发现的问题

见 `ISSUES.md`。

## 下一件事

按 `README.md` 的边界，SKILL 接进 `ToolRegistry`（**已完成**）、铺 `rmcp`
（**已完成**）、`with_mcp_tools` + `tools/call`（**已完成**）—— 三步都走完了，
**可以开始跑这 100 条**了。顺序反过来的话，测出来的全是「功能还没做」，
而不是真 bug。

跑之前要知道的三件事：

1. **浏览器那条路还没走过一次。** 之前几轮每轮开新对话、没在第一条 skill 调用里
   执行 `skill(name="browser-use:control-in-app-browser")`，于是 Browser 一次都没
   操作成。**跑之前先在对话的第一条 skill 调用里加载它**，否则任何 Browser 调用
   都会直接返回 `SKILL_REQUIRED`。
2. **界面与服务端都要起。** 后端 `cargo run -p quill-server`，前端 `ui/web` 下
   `npm run dev`（`vite.config.ts` 把 `/api` 代理到 `QUILL_BACKEND`，默认
   `http://127.0.0.1:18777`）。**WSL 里没有 node，前端只能在 Windows 侧起。**
3. **判「链路是否通」时先看工具表。** MCP 与 SKILL 现在都真的挂进对话工具表了，
   所以一条任务失败时，先分清是「工具没挂上」还是「挂上了但模型没调/调错」。
   设备页与 `GET /api/extensions/{mcp,skills}` 都会如实报
   `mounted` / `model_can_see`，**不要**从 `servers.length` 或 `tool_count` 推断。

### 已知代价（不是 bug，但会感觉到）

**每发一条消息，都会把用户配的 MCP 服务器重新拉起来一遍**（`with_mcp_tools`
每次对话都真的握手，不缓存工具列表）。本地 stdio 服务器通常是几十到几百毫秒，
但一台慢启动的会给每条消息加上它的启动时间。真到扛不住时再上带 TTL 的缓存，
**并且界面上要显示缓存年龄** —— 否则「界面上说挂了几个」又会与模型实际拿到的不一致，
那正是这轮刚修掉的那类谎。
