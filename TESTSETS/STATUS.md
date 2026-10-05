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
**「quill 从哪知道模型真实窗口」** —— 这一条已经查清并修好（ISSUE-022）：

- **能知道**：`GET /v1/models` 真的报了 `meta.n_ctx = 8192`（llama.cpp 扩展），
  而 `GET /api/admin/providers/{id}/models` 也一直在读它。**能力本来就有。**
- **但读错了字段**：`CONTEXT_KEYS` 把 `n_ctx_train`（训练窗口 262144）
  排在第一位，于是探测报出**大 32 倍**的假窗口，界面照着显示、
  `validate()` 还教用户照它填 `max_context_tokens`。**已修**（ISSUE-022）。
- 附带发现：这条错误**是被一条测试明确保护着的**（旧测试名
  `..._prefers_the_trained_length_...`），所以跑测试发现不了，
  是真机跑任务看到 8192 vs 32768 才顺藤摸出来的。

因此 ISSUE-019 的修复方向**重新可行**：现在能查到真实窗口（8192），
预检应当用**探测到的窗口**，而不是配置值（32768 是用户填的，会骗过预检）。

**但在那之前先补了一处界面自相矛盾（ISSUE-023，已修）**：模型页上 provider 卡片
把配置值 32768 标成「上下文」，模型标签却把探测值 8192 也标成「上下文」——
同一页面、同一个 provider、两个数、两个一样的标签，谁也不说谁。
现在卡片改成「配置上下文」，分组下新增一条对账提示：
配置 > 探测时明写两个数并给出下一步（改配置，或调大模型服务的 `-c`）。
判断规则刻意是「只报配置>探测；探测值缺失不报」——
「不知道」不等于「一致」，那也是另一种谎。新增 7 条单测（75 passed），
把比较方向反过来后 3 条立刻变红，证明不是空断言。

**下一件该做的事**（本轮未做，写清楚免得下轮重想）：给 `/healthz` 加一个
**实测**的窗口字段，让聊天页也能看到 8192 而不是只有配置值。但 `/healthz`
被前端轮询，**不能**在请求里同步探活（ISSUE-016 的教训），
得配带 TTL 的缓存，且**缓存年龄要在界面上显示**。

## 「100 条」这个数本身曾经是错的（2026-10-06，ISSUE-024）

按 id 取任务时发现 `atlas-689bd255c042` 重复。全量一数：
**100 条里只有 66 个唯一 id**，10 组冲突，最多的一组有 **9 条不同的任务**
（prompt、`required_tools`、依赖的服务器全都不同）。

根因在 `build_tasks.py:53`：MCP-Atlas 的 `TASK` 是 24 位十六进制，
**前 12 位是分组前缀、后 12 位才是序号**，而当时截断到 12 位 ——
**恰好只留下分组前缀、把唯一的那半截扔了**。原始 500 行 `TASK` 全部唯一，
截断后只剩 32 个 id。**不是哈希碰撞，是会稳定复现的截断错误。**

**已修**：用整个 `TASK` 做 id，并加了一条**唯一性断言** ——
id 冲突就如实报错、**不写 tasks.json**。验证过断言不是摆设：
退回截断后 `rc=1` 且 `tasks.json` 的 md5 没变。
重新生成后 **100 条 / 100 唯一 id**，`TESTSETS/skills/` 逐字节未动。

**这件事为什么重要**：在它修好之前，「按 id 记进度」会**虚增**，
「按 id 跑某一条」会跑到**另一条**身上（实测给了 13 个 id、实际跑了 34 条）。
所以 **ISSUE-024 修好之前，任何按 id 统计的进度数字都不可信** ——
包括本文件里此前写的 0/100。现在起，100 条是真的 100 条。

## 下一轮该修的：runner 不做任务隔离（ISSUE-025）

实测发现跑第 7 条（MCP-Atlas 任务，不需要任何技能）时，模型在调
`csv-processing` —— 那是前几轮跑 SkillsBench 时挂上去、**一直没撤**的技能。
库里现在挂着 10 个 SkillsBench 技能 + 一个更早那轮的 `notes` stub。

**这不是 quill 的 bug**：技能与服务器是按用户配置、对所有会话生效，
那是产品设计。脏的是 runner —— 它只「挂」，从不「撤」。
影响双向：撑大每条请求的输入（放大 ISSUE-019）、带偏模型去调不该调的技能、
让批量结果彼此不可比。

修的时候要注意边界：收敛到本条任务需要的集合，跑完**恢复原状**，
**不要**「用完就删」——那会毁掉用户真实的配置。

## 修它之前先撞出一条真 bug（ISSUE-026，已修）

去查「MCP 能不能停用」才发现：`mcp_repo` 有 `enabled` 列、POST 也读它，
但 `with_mcp_tools` **一行都没判** —— 用户停用了服务器，工具照样挂进对话。
同一条路径上的**技能**是尊重 `enabled` 的，所以这是两条路径待遇不一致。

这意味着 ISSUE-025 想做的「MCP 按任务隔离」在产品侧**根本做不到**。已修：

- `with_mcp_tools` 过滤掉 `enabled=false` 的行，全停用时**不去握手**。
- `api_extensions::mcp_body` 用**同一个**过滤函数 —— **只改前一处会造出
  「界面说 mounted=3、模型一个都调不到」**，那正是 ISSUE-014 那一类的谎。
  停用的那台在 `status` 里如实报「已停用 + mounted=0」而**不抹掉**
  （抹掉用户会以为被删了）；`servers` 仍返回全部行，否则开不回来。

真机验过：`停用前 mounted=3` → `停用后 probed=False / connected=False /
mounted=0 / disabled=True`，带中文「下一步」；验完已恢复 `enabled=true`。

**还没做**：前端 `McpServerConfig` **根本没有 `enabled` 字段**，
所以用户仍然没法在界面上停用一台 MCP 服务器，只能走 API。
下一轮可以把这两件一起做掉：前端开关 + `runner.py` 的任务隔离。

## runner 任务隔离已做（ISSUE-025，已修并真机验过）

`runner.py` 现在：整轮跑前拍快照 → 每条任务**跑之前**把配置收敛到
**这条真正需要的集合** → 整轮跑完**原样恢复**。刻意**不做**「用完就删」——
那会毁掉用户真实配置，这即便是专用测试库也该守住的边界。
收不干净 / 恢复失败都**必须喊出来**，不留一份改坏的配置给下一轮。

真机跑 3 条技能集合互不相同的任务（`/tmp/quill-iso.jsonl`）：

| 任务 | 声明的技能 | 停用了几个残留 | 收不干净 |
|---|---|---|---|
| `sb-3d-scan-calc` | `mesh-analysis` | 9 | 无 |
| `sb-bike-rebalance` | 4 个（全新） | 1 | 无 |
| `sb-citation-check` | `citation-management` | 4 | 无 |

跑之前 10 个技能可见，跑之后**仍是同样 10 个** —— 恢复成功。

**归属更正**：这条一开始被我记成「纯 runner 的问题」，只对了一半 ——
MCP 那一半**当时做不了**，是因为 quill 侧 `enabled` 只存不用（ISSUE-026）。

## 顺带又撞出一条（ISSUE-027，待修）

`sb-3d-scan-calc` / `sb-citation-check` 报的是
`provider_unavailable / 模型连续 4 轮都在请求调用工具，没有给出正文`。

**detail 写得很对**（如实说了发生了什么 + 已执行的工具 + 「换个更直接的问法」），
**错的是结构化的 `next_step`**：`api_chat.rs:598` 用了 `service_unavailable`，
于是附上了那句固定串「确认端点活着 / 启动 llama-server」——
而这 4 轮里模型每次都**回话了**，只是没收敛，服务好得很。
与 ISSUE-018 同一族，且 `ProviderRejected` 的建议在这里也不合适
（什么都没报错）。**4B 打转本身不算 bug**，算 bug 的是建议指错了地方。

**已修**：新增 `ApiError::ToolLoopExhausted { detail, advice }` ——
「连不上 / 被拒 / 不收敛」是三件不同的事，错误码分成
`provider_unavailable` / `provider_rejected` / `tool_loop_exhausted`。
status 仍是 503（确实没拿到正文）。**detail 一字未改**（它本来就写得对），
只把结构化 `next_step` 换成与它同向的那句。967 passed / 0 failed。

## 50 条 SkillsBench 全量跑（2026-10-06，本轮进行中）

隔离修好之后，这是第一次**大批量**跑。结果写在 `/tmp/quill-sb50.jsonl`。
**注意：这批跑的是修 ISSUE-027 之前的二进制**，所以其中「模型连续 4 轮
都在请求调用工具」那几条仍报 `provider_unavailable` —— 重跑之后应是
`tool_loop_exhausted`。**这不是数据矛盾，是二进制版本差异，已记在此。**

已经能看出来的两点：

1. **隔离立刻见效**：`sb-ada-bathroom-plan-repair` 之前因为上下文溢出判 FAIL
   （9804 token > 8192），隔离掉多余技能之后判 **UNJUDGEABLE** ——
   链路通了，只是三个维度都判不了。**同一个任务，环境一干净就过��。**
2. 失败原因**高度集中**在两类：`provider_rejected`（上下文超了）与
   「模型连续 4 轮只调工具不给正文」。后者占多数，且已执行的工具里
   **反复出现 `list_experts`** —— 这是内置工具，值得单独查一下
   是不是它把模型带进了循环（尚未成 ISSUE，先记在这里观察）。
3. **新增判定 `TIMEOUT`**（ISSUE-028，已修）：`sb-crystallographic-wyckoff-position-analysis`
   报了 `err=transport / timed out`，而它被算成了 `FAIL` ——
   **等于「我自己等不下去」被记成「quill 跑完了、只是没通过」，而 FAIL 计入跑过。**
   账算得出来：quill 单次调用超时 300s、一轮最多 1+4 次串行 → 最坏 1500s，
   而 runner 当时只等 180s。已加 `TIMEOUT` 判定（不计入跑过），
   默认超时放大到 600s 且可调。**这类虚增是系统性的**（工具循环越多越容易命中）。

**这一批还没跑完**（本轮到 17/50），所以上面的统计口径都还没定稿。
跑完之后再据此更新这里的进度数字 —— 在那之前不写任何进度百分比。

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
