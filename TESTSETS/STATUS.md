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
- [ ] **下一步**：`ToolRegistry::with_mcp_tools` —— 把 `tools/list` 的条目挂进对话工具表。
      MCP 工具名常用连字符（`read-file`），而 `skills` 的 `name` 有
      `NOT GLOB '*[^a-z0-9-]*'` 的 CHECK，所以挂载时要自己定「挂上去叫什么」，
      并复用 `tools::skill_visibility` 那种「同名就跳过而不是顶掉」的守卫。
- [x] 全量门禁 0 failed —— 2026-10-06：`cargo test --workspace` 57 个目标
      940 passed / 0 failed、`ui/web` 62 passed、`typecheck` 干净、`npm run build` 成功、
      `i18n-check` 0 问题、`library-check` 334/334。
      这一轮的 940 是**修好门禁脚本之后**数出来的 —— 旧脚本的 `failed` 恒为 0，
      之前那些「0 failed」不算数。见 ISSUE-013。

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

按 `README.md` 的边界，先把 SKILL 接进 `ToolRegistry`（**已完成**），再铺 `rmcp`
（**已完成**），之后才跑这 100 条。顺序反了的话，测出来的全是「功能还没做」，
而不是真 bug。

**`rmcp` 已铺完，`with_mcp_tools` 还没写。** 现在的状态是：MCP 服务器能真的连上、
`tools/list` 真的拿到了，界面也如实显示了 —— 但那些工具**还没进对话的工具表**，
模型这一轮仍然调不到（`note` 里就是这么说的，不许改成听起来更好的说法）。

下一件事就是在 `with_skills` 旁边补 `with_mcp_tools`：MCP 服务器 `tools/list`
返回的条目走同一个 `ToolRegistry`，所以「内置的」与「MCP 来的」在模型看来没有区别。
三处要一起想清楚，别只做其中一处：

1. **挂上去叫什么。** MCP 工具名按规范允许 `a-zA-Z0-9_-` 与点，而 `quill_provider`
   侧的工具名要过本地 4B 模型的函数名限制；`skills.name` 又有
   `NOT GLOB '*[^a-z0-9-]*'`。三套规则不重合，得定一个映射并在界面上显示原名。
2. **同名怎么办。** 与内置工具（`list_experts`）或别的 MCP 服务器撞名时，
   要**跳过并报原因**，不能像 `ToolRegistry::register` 那样静默顶掉。
3. **执行体是同步的。** `ToolHandler` 是 `Fn(&Value) -> Result<String, String>`，
   而 `tools/call` 是 async 且要过一遍超时与 `max_concurrent_calls`。
   这里需要一个桥（阻塞等待 / 或改 `ToolHandler` 的签名），选哪个都要写清楚理由。

这三件事没定清楚之前就开始写 `with_mcp_tools`，多半会写出一个「工具挂上了、
一调就超时」的版本 —— 那比现在诚实地说「还没挂」更糟。
