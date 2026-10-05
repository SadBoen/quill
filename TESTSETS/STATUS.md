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

**进度仍是 0/100，一条都没标跑过。** 下面「这一轮验到了什么」记的是**链路本身**
的验证结果，不是那 100 条里任何一条的完成情况 —— 不要拿它当进度。

## 这一轮验到了什么（2026-10-06，真机、真子进程、非 mock）

用 `.wsl-run-browser.sh` 起了一套**独立的临时实例**（库在
`/tmp/quill-browser-round/quill.db`，不碰仓库里那份；令牌 `dev-token` → 用户
`@alice`），对着它把两条链路各走通一遍：

**MCP 那条**（`quill-mcp-stub` 是一个真的会说话的 stdio 子进程）：
1. `POST /api/extensions/mcp` 配一台服务器 → 服务端真的 `initialize` + `tools/list`
2. 回包 `connected=true  probed=1  mounted_count=3`，且
   `server_declares_tools=true  protocol=2025-06-18  server_info=quill-test-stub 1.0.0`
3. 挂载名 ↔ 原名一一对应：`notes__read-note ← read-note`（另两个同理）
4. 发一条消息 → **模型真的调了** `notes__read-note`，参数
   `{"path": "a.md"}`，返回 `stub 执行了 read-note，收到参数 {"path":"a.md"}`
   —— 参数真的过了线（stub 是回显参数的，所以这是**观察到的**，不是「没报错」推断的）
5. 模型据工具结果给出了正文：`笔记 a.md 的内容是："stub 执行了 read-note…"`
6. 该条消息真的存进了库（`role=assistant`，正文非空）

**SKILL 那条**（用真实的 benchmark 技能 `TESTSETS/skills/text-parser.md`，
SkillsBench v1.1 / Apache-2.0，916 字节）：
1. 导入后 `GET /api/extensions/skills` 报 `model_can_see=true`、`content_chars=916`
2. 发一条消息 → **模型真的调了** `text-parser` 工具，参数带上了原文
3. 模型按这套方法给出了正确的键值对结果

**浏览器那条**：`navigate` + `inspect` 都通了（quill 界面打开、登录页读到了），
**卡在登录**：运行时把登录表单判为「需要用户接管」，不许我自己填任何账号/令牌，
已就此向你提问。**登录之后浏览器回归才算真正走通。**

## 用 `runner.py` 真跑了 3 条（2026-10-06，本轮）

`--dry-run` 全量先跑了一遍：**120 个 skill slug 与 `TESTSETS/skills/*.md` 完全对齐**
（0 缺 0 多），并逐条报出缺哪几台 MCP 服务器。然后真跑了 3 条 SkillsBench：

| 任务 | 判定 | 观测到的 |
|---|---|---|
| `sb-3d-scan-calc` | `UNJUDGEABLE` | 链路通；模型真的调了 `notes__read-note` 与 `mesh-analysis`，两个都 `ok=true` |
| `sb-ada-bathroom-plan-repair` | `FAIL` | 请求 9804 token > 8192 上下文，压根没发出去（见 ISSUE-018 / 019） |
| `sb-adaptive-cruise-control` | `FAIL` | 同上，8525 token > 8192 |

**进度仍是 0/100**，这三条**不计入**：跑不完不是代码问题，是缺外部凭据与基准夹具
（下一节有实测数字）。上面每条都给了观测值，没有一条是靠「没报错」推断通过的。

**这一轮最值钱的产出是 `runner.py` 自己暴露的两个洞**：

1. **假通过**：`verdict()` 曾把「三个维度全部 unjudgeable」判成 **PASS** ——
   「什么都没验成」被算成通过。已修成独立的 `UNJUDGEABLE`，
   「真的跑完」只认 `PASS + PARTIAL + FAIL`。`sb-3d-scan-calc` 修前 PASS、修后
   `UNJUDGEABLE`，差的就是这个洞。
2. **错误建议**：模型**已经回过话**（回的是 HTTP 400 `exceed_context_size_error`），
   quill 却说「确认端点活着 / 启动 llama-server」。端点是活的，它刚结构化地回了 400。
   已修并在真机上复跑验过：错误码 `provider_unavailable` → `provider_rejected`，
   建议换成「不要去重启模型服务，它正在正常应答」（ISSUE-018）。
3. **两条「下一步」打架**：detail 里还嵌着一句旧的含糊建议，和结构化的
   `next_step` 一起渲染成两个段落（ISSUE-020，**已修**：detail 改用
   `ProviderError::message()`，不再拼那句 tail；`Display` 行为一个字没动，
   免得流式 / CLI 那些没有 `next_step` 字段可用的调用点丢掉下一步）。

**顺带查清、并且不算 bug 的一件事**：`/healthz` 报的
`compaction_threshold_tokens=8000` 看着像「设了阈值却没拦住 8525 的请求」，
但前端 `ChatPage.tsx` **明写了**「已配置压缩阈值，但压缩还没实现：
超过上限不会自动摘要，需要自己新建会话」，仓库里也确实没有压缩实现。
那是**诚实披露**，不是谎 —— 本地没有拦截点这件事，界面已经告诉了用户。

**但同一块 UI 里有另一处是真谎（ISSUE-021，已修）**：它把
`max_context_tokens=32768` 直接印成「上下文上限」。而同一个部署里模型自报
`n_ctx: 8192` —— 32768 是配置值，不是实测值，差 4 倍。
本机实测新文案已进 `dist`（`npm run build` 后逐字核对）。
顺带作废了 ISSUE-019 原定的修复方向：**拿 32768 做 token 预检在当前部署里
根本不会触发**（8525 < 32768，预检放行、上游照样拒）。

**下一件该做的事因此变了**：不是「加预检」，而是先解决
**「quill 从哪知道模型真实窗口」** —— 要么 `provider.models()` 之后另找一条
能报窗口的途径，要么在配置页明确告诉用户「这个数得你按模型实际情况填，
quill 不会去核对」。

## 100 条为什么在这里跑不完（实测，不是猜测）

数清楚了，两段各有各的硬缺口：

- **MCP-Atlas 50 条**：`required_tools` 指向 **22 台**服务器
  （github 96 次、mongodb 51、filesystem 47、git 44、airtable 42、notion 26、
  slack 20、twelvedata 19、whois 18、oxylabs 18、wikipedia 18、pubmed 16、
  weather 12、context7 12、fetch 9、calculator 9、alchemy 9 …）。
  仓库里**既没有这些服务器的程序，也没有它们的连接配置与凭据**：
  `_raw/mcp-atlas-*.json` 每一行只有 `TASK / ENABLED_TOOLS / PROMPT /
  GTFA_CLAIMS / TRAJECTORY` 五个字段，**没有 command、没有 url、没有 env、
  没有 token**。而任务本身是**复合**的 —— 比如第 1 条要 airtable + github +
  fetch + git + calculator + filesystem 一起，缺任何一台都答不出
  `expected_claims`。
- **SkillsBench 50 条**：`required_tools` **一条外部服务器工具都不需要**
  （数过了：50/50 全是空的），技能正文也齐（`TESTSETS/skills/` 120 份真件）。
  **但任务的输入夹具不在仓库里**：每个任务的 prompt 都指向
  `/root/input/input.pdf`、`/root/scan_data.stl` 之类的文件，而
  `_raw/skillsbench/*/environment/` 下**只有 `skills/`，没有任何输入数据**。
  例如 `sb-edit-pdf` 要的 `/root/input/input.pdf` 与 `input.txt` 都不存在。

**结论**：按字面「100 条全部跑完」这个完成条件，在当前环境里**做不到** ——
缺的是外部凭据与基准夹具，不是代码。**不要**用 stub 服务器冒充 Airtable 去把
MCP-Atlas 那 50 条标成跑过 —— 那正是本项目最不能出的错。
按 `README.md` 自己写的边界（「用它们当真实用户场景去打 quill 的界面与对话链路」，
「找的是 quill 自己的问题，不是刷 MCP-Atlas 的分数」），这 100 条的**prompt**
仍然完全可用：拿来压界面与对话链路、找契约漂移/状态不一致/错误吞掉/隔离失效。
**这个用法不需要那 36 台服务器。** 下一步该走哪条，需要你定（见「下一件事」）。

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

**需要你定的一件事：那 100 条按哪种口径算「跑过」。**

三步接线（SKILL → `ToolRegistry`、`rmcp`、`with_mcp_tools` + `tools/call`）已全部完成并
在真机上验通，所以「已打通 → 开始跑任务」这个条件是满足的。但按字面「100 条全部跑完、
进度到 100/100」在当前环境里做不到（缺口见上一节：外部凭据与基准夹具都不在仓库里）。

请从下面三条里挑一条：

1. **按 README 自己写的边界跑**（推荐）：拿这 100 条的 **prompt** 当真实用户场景，
   压界面与对话链路，找 **quill 自己的**问题（契约漂移、状态显示不一致、错误吞掉、
   隔离失效）。**不需要那 36 台服务器**，也不需要基准夹具 ——
   缺工具时如实记「这条依赖的服务器没配，工具类断言无法判定，但链路本身验了」。
   这种口径下 100 条都能跑完。
2. **只跑 SkillsBench 那 50 条**：同样缺输入夹具，但**至少技能正文齐**，
   可以先只验「SKILL 有没有挂上、模型有没有调」这一层，答案正确性不判。跑完是 50/100。
3. **先补齐外部依赖**：你提供 Airtable / GitHub / Notion / Slack / Twelvedata 等
   凭据，以及 SkillsBench 的输入夹具文件，我再按字面口径跑满 100 条。
   在这之前进度只能停在 0/100。

**不要**用 `quill-mcp-stub` 之类冒充 Airtable 去把 MCP-Atlas 那 50 条标成跑过 ——
界面显示「模型调得到」而实际调的是一个只会回显的子进程，那正是本项目最不能出的错。

**`runner.py` 已经写出来了**（2026-10-06，ISSUE-017 已修）。上面三条不管选哪条，
逐条驱动都不用再从零写：纯标准库，参数
`--base/--token/--out/--limit/--only/--source/--dry-run`，直接打
`/api/extensions/*` 与 `/api/sessions/*`。用法见 `TESTSETS/runner.py` 顶部说明。
2026-10-06 那几轮验证用的起服务脚本仍是本机临时脚本，
**被 `.gitignore` 的 `/.wsl-*.sh` 规则挡在仓库外**，别指望 clone 下来就有 ——
需要的话照着 STATUS.md 里记的步骤重写。

### 跑之前要知道的三件事（2026-10-06 实测更新）

1. **浏览器那条路走到登录页为止。** `navigate` 与 `inspect` 都通了，
   但登录表单被运行时判为「需要用户接管」，我不能自己填。**你登录之后**，
   浏览器回归才算走通；或者你授权我用那个 `dev-token`。
2. **界面与服务端怎么起**（`.wsl-run-browser.sh` 已经把这套踩过的坑都填上了）：
   - 前端**不需要**单独起：`quill-server` 直接托管 `ui/web/dist`，
     先 `npm run build` 即可（`vite.config.ts` 那套 dev proxy 只在 `npm run dev` 时用）。
   - **`QUILL_LLM_BASE_URL` 别用 `127.0.0.1`。** 4B 模型跑在 Windows 上、
     quill-server 跑在 WSL 里时，WSL 的 `127.0.0.1` 不是 Windows 的那个，
     会得到「连接被拒绝，目标端口上没有进程在听」。用
     `ip route show default | awk '{print $3}'` 取网关地址。**这个地址随 WSL 重启变**。
   - **改完 `QUILL_LLM_BASE_URL` 必须删掉临时库重启。** 首次启动会把 provider
     配置**写进库**，之后环境变量就不再生效 —— 不删库的话，新进程日志里看着是新地址，
     `/healthz` 报出来的却还是库里那个旧的。见 ISSUE-016。
   - **停服务要连子进程一起停。** `task_stop` 停的是 `cargo`，`quill-server`
     子进程还活着并占着 8848；不 `pkill -f target/debug/quill-server` 的话，
     下一次启动「看起来成功」，但 8848 上回答的还是那个旧进程。
3. **判「链路是否通」时先看工具表。** MCP 与 SKILL 现在都真的挂进对话工具表了，
   设备页与 `GET /api/extensions/{mcp,skills}` 会如实报 `mounted` / `model_can_see`，
   **不要**从 `servers.length` 或 `tool_count` 推断。

### 已知代价（不是 bug，但会感觉到）

**每发一条消息，都会把用户配的 MCP 服务器重新拉起来一遍**（`with_mcp_tools`
每次对话都真的握手，不缓存工具列表）。本地 stdio 服务器通常是几十到几百毫秒，
但一台慢启动的会给每条消息加上它的启动时间。真到扛不住时再上带 TTL 的缓存，
**并且界面上要显示缓存年龄** —— 否则「界面上说挂了几个」又会与模型实际拿到的不一致，
那正是 ISSUE-014 那一类谎。
