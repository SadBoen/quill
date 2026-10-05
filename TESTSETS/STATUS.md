# 测试集进度表

100 条真实任务（50 MCP-Atlas + 50 SkillsBench），来源与许可见 `README.md`。
生成时间：2026-10-06。

## 前置条件（不满足就别开始跑）

- [x] SKILL 已通过 `skills_repo::as_tool_spec` 挂进 `ToolRegistry`（对话里能真的调）
      —— 2026-10-06 完成。`ToolRegistry::with_skills`（async，行在库里、正文在磁盘上），
      `api_chat.rs` 构造完 registry 就调。只挂 `enabled` 的；磁盘没正文的跳过而不是
      注册空工具；与已有工具同名的跳过而不是顶掉它；查库失败**整条请求失败**，
      不静默降级成「只有内置工具」。用户过滤复用 `skills_repo::list` 的 `user_id`。
- [ ] MCP 协议层已接（`rmcp`），`GET /api/extensions/mcp` 的 `connected` 不再恒为 false
- [x] 全量门禁 0 failed —— 2026-10-06：`cargo test --workspace` 907 passed / 0 failed、
      `ui/web` 60 passed、`i18n-check` 0 问题、`library-check` 334/334

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

按 `README.md` 的边界，先把 SKILL 接进 `ToolRegistry`（**已完成**），再铺 `rmcp`，
之后才跑这 100 条。顺序反了的话，测出来的全是「功能还没做」，
而不是真 bug。

`rmcp` 铺完之后，`with_skills` 旁边要补 `with_mcp_tools` —— MCP 服务器
`tools/list` 返回的条目走同一个 `ToolRegistry`，所以「内置的」与「MCP 来的」
在模型看来没有区别。
