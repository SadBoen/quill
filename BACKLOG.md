# BACKLOG —— 按里程碑分组的待办明细

**事实基线**：`TESTSETS/STATUS.md`（真机跑过的进度）与本文每条后面括号里的 `file:line`。
**归属**：每条都挂在 `MILESTONES.md` 的一个里程碑下；里程碑没列的条目不许进来。
**过期处理**：发现与代码不符就改这里，不要在别处留快照 —— 本文件以前就是重灾区。

> 2026-10-06 重构说明：旧版这份文件**自相矛盾** —— 前半段的审计结论（28 条 501、MCP 协议层待办）
> 被同一文件后半段与 `TESTSETS/STATUS.md` 推翻，却没有删掉，于是照着它排期会把已做完的事重做一遍。
> 本版删掉了所有被推翻的说法，并给每条加上证据位置。

---

## M0 · 可重复的地基

### B0-1 门禁解析曾经恒为「通过」 ✅ 已修，但要防复发

`.scripts/gate-selftest.sh` 的文件头记着这件事：旧门禁用 `awk -F'[ ;]'` 数失败，
而 `-F'[ ;]'` 把 `"; "` 当两个分隔符、多出一个空字段，失败数其实在 `$7`。
于是 `$6` 恒为空，`failed` 恒等于 0 —— **跑多少个红都印「通过」**。
现版解析与自测逐字锁在一起（`.scripts/gates.sh` 与 `gate-selftest.sh` 同源）。

**残留**：~~`gates.sh` 本身还没在 WSL 侧实跑过~~ —— **2026-10-06 已在 WSL 侧实跑通过，
退出 0**。见 B0-2。

### B0-2 `gates.sh` 需要一个能跑的落点 ✅ 已通

M0 的判据就是 `bash .scripts/gates.sh` 退出 0。原先两侧各缺一半：
Rust 工具链在 WSL2，node/npm 只在 Windows 侧。

**已解决**：WSL 里装了免 root 的 Node LTS（官方二进制包解压到 `~/.local/node`，
先用官方 SHASUMS256 校验过），`gates.sh` 在非交互 shell 里会主动把它挂进 PATH
（只写 `~/.bashrc` 没用 —— `bash gates.sh` 既不读 .bashrc 也不读 .profile）。
现在 **`bash .scripts/gates.sh` 在 WSL 侧一条命令跑完全部门禁，退出 0**。

**途中撞到并修掉的两个真问题**：

1. `.i18n-check.mjs` 把路径写死成 `D:/96_CoderWorld/quill/...`，在 WSL 里 ENOENT。
   改成按脚本自身位置推 —— 一道门禁只能在一台机器的一个目录上跑，
   那它守的不是仓库，是那台机器。
2. `node_modules` 在 D: 上被 Windows 与 WSL **共用同一份**，但 rolldown 的原生
   二进制每个平台一个目录名。任一侧单独 `npm install` 都会把另一侧的删掉，
   另一侧启动就报 "Cannot find native binding"。
   试过「两个包装在一起」—— Windows 因 libc 不符直接拒装；
   试过 `--force` —— 装上了，但下一次普通 `npm install` 又被剪掉；
   也不能写进 `package.json` —— 那会让 Windows 侧 `npm install` 因装不了 glibc
   包而直接失败，等于为了 WSL 把 Windows 弄坏。
   **结论：一份 node_modules 只能服务一个平台**，这是结构事实不是配置问题。
   `gates.sh` 因此在跑前端前先真的 `import('rolldown')` 探一次，
   加载不了就按当前平台补装（幂等，几秒）。两侧都验过能自愈。

   **要记住的代价**：`node_modules` 只有一份，两侧共用，所以**一次只能服务一个平台**。
   在 WSL 跑完门禁之后，Windows 侧再跑前端会看到
   `Cannot find native binding` —— 那是 WSL 刚把 win32 的包剪掉了。
   在 Windows 侧跑一次 `npm install` 就回来（反之亦然）。
   这不是坏了，是这个结构的必然结果；`gates.sh` 自己会补，直接手跑就要自己补。

### B0-3 前端 lint 从来没跑起来 ✅ 已通（2026-10-07）

`ui/web` 下不存在任何 `eslint.config.*`，`npm run lint` 直接退出 2 ——
既有问题，非某一轮引入。后果是 `react-hooks/exhaustive-deps` 这条规则
**从未生效过**，而 `EChart.tsx` 的 `useEffect` 依赖正好是它该管的。

**已解决**：`eslint.config.js` 已入库（依赖本来就在，缺的只是那份配置，
不需要改依赖政策）。`npx eslint .` 现在 **0 error、11 warning**。

首次真跑就抓出 2 条 error，两条都是真的派生状态问题，已修：

| 位置 | 问题 | 修法 |
|---|---|---|
| `MemoryPage.tsx` | effect 里 `setPaths` + `setCurrentPath`，而这两个值都能从 `list.data` 算出来 | 改成 `useMemo` + 纯推导的 `activePath`，effect 整个删掉 |
| `ChatPage.tsx` | effect 里同步 `setHistory(null)` 擦换会话的画面 | 搬到渲染期判断，**并在发送开始时同步 `setHistoryFor(新会话)`** |

第二条有个坑值得记下来：只把擦除搬出 effect 是不够的。从 /chat 直接发第一条
消息时，`navigate` 换掉 `sessionId` 与「开始发送」发生在同一轮；等流结束
`sending` 变 false，而 `historyFor` 若还停在旧值，条件成立，**刚流完的那一屏
被当成「上个会话的残留」擦掉** —— 用户看着刚收到的答案凭空消失。
是 `ChatPage.test.tsx` 当场逮到的（`工具往返那一轮的正文会被抹掉` 那条变红）。
搬的时候必须一起搬归属，不能只搬动作。

剩下 11 条 warning 不影响退出码，按需处理：`react-refresh/only-export-components`
（组件文件里顺带导出常量/函数，影响 Fast Refresh）与两条 `exhaustive-deps`
（`LibraryTab.tsx` / `TeamsTab.tsx` 里 `useMemo` 的初始化表达式每次渲染都新造一个数组）。

> `gates.sh` **不含 lint**。别把「门禁退出 0」当成「lint 也过」的证据。

### B0-4 清理本地噪音 ✅

旧 `.gitignore` 只忽略 `/.wsl-*.sh`，同批产出的 `.wsl-*.py`、`.wsl-commitmsg*.txt`
与各类 `.out`/`.log` 全在 `git status` 里裸奔。已放宽为 `/.wsl-*` 等五条规则，
`git status --porcelain` 从 20+ 条噪音降到 0。

### B0-5 三个空壳 crate：建了但没人用 ⚠ → ✅ 已删（2026-10-07）

`quill-bridge`（9 行）、`quill-ext-hub`（7 行）、`quill-xtask`（13 行）各只有一个
`assert_eq!(2 + 2, 4)` 的「能编译」测试，全仓库**没有任何 crate 依赖它们**，
`*.md` 里也从没被提到过 —— 建了但没人用、没人引、没人写。

留着比缺失更糟，两个实际害处：

1. **假的防护**。`quill-bridge` 的 description 写的是「实例间委派共享防护
   （SSRF / 路径白名单 / 头剥离）」，`quill-ext-hub` 写的是「MCP / SKILL /
   插件注册中心（配置存服务端，铁律）」。读 `Cargo.toml` 的人会以为这些安全
   边界已经存在 —— 而代码里一行都没有。真要实现时，这层「看起来已经在了」
   会让人以为不用做。
2. **假的覆盖面**。三个占位测试让 `cargo test` 的数字更好看，却什么都没验证
   —— 正是 M0 记的那类「测试全绿本身不是判据」。

已从 `Cargo.toml` 的 `members` 移除并删目录，`cargo build --workspace` 通过。
真需要其中某个能力时，按真实需要重建，并给它真测试与真调用方。

---

## M1 · 单人闭环

### B1-1 20 条 501 桩，其中这些界面有真按钮 🔴

`routes.rs` 里 `not_implemented(` 的桩共 **20 条**（旧文档写 28，实际不是）。
界面上点了必失败的：

| 界面入口 | 打到哪 | 后端有没有能力 |
|---|---|---|
| 实例 / 用户管理页 | `GET/POST /api/users`、`PATCH/DELETE /api/users/{id}` | **已通一半**（2026-10-06）：`GET` 真读库并支持真分页，`PATCH` 真启停。`POST`/`DELETE` **有意保持 501** —— 账号只来自部署配置，软删除只做了一半。旧的「三层缺两层」说法已过期：`ControlPlane` 的 `list_users`/`set_user_status`/`create_user`/`create_invite` 一直都在，缺的是 HTTP 那层 |
| 设备页 + 管理页的 MCP 卡片 | `PATCH /api/extensions/mcp/{name}` | `routes.rs` 里仍是桩；但 `GET/POST /api/extensions/mcp` **是真实现**（旧文档把它也算成桩，错了） |
| 工作区页 | `POST /api/backup/export|verify`、三条 `/api/upgrade/*` | backup 能力齐（见 M4），upgrade 只有 120 行守卫 |

**为什么排 M1**：这些是"真按钮、真失败"，不是假开关，但也不产生价值。
建议顺序：备份接线（M4 的实现已在，最快见效）→ 用户管理三层补齐。

### B1-1b 停用曾经挡不住 `QUILL_TOKENS` 🟢 已修

**这一条做完了。** 早先 `CompositeTokenResolver::resolve` 先查环境变量令牌表、
命中直接放行、**完全不查库**，于是把某人状态改成 `disabled` 之后他照样进得来——
管理界面上的「已停用」是在骗运维，而接口还配了一句 `warning` 去把这个洞合法化。

判据是用户那句「octop 怎么做，我们就也怎么做」。核 octop 的结论：它只有**一种**
令牌（登录换来的 JWT），服务端是唯一裁判，而且明确拒绝提供 `disableAuth`
这类开关（`dashboard/src/api/modules/auth.ts:306-308`）。所以差距不在
「有没有第二条路」，而在「令牌解析有没有回库」。于是照 octop 的做法改：

- `ControlPlane::authz_of(id)`（`service.rs:687`）：按 id 读 `role`/`status`，
  **不带权限检查** —— 鉴权时调用者还没有身份，不能走 `get_user` 的 owner 校验。
- `CompositeTokenResolver::confirm_identity`（`auth.rs`）：环境变量令牌命中之后
  必须回 `users` 行核状态与角色。行不存在、已软删、已停用，一律按令牌无效处理。
- 角色**以行为准**，不再采信配置里写的 `:admin`。`:admin` 只在启动引导时决定
  初始角色；之后能改角色的是管理界面，鉴权必须跟它一致，否则「降级」是假的。

代价是环境变量令牌在热路径上多一次 DB 查询。这是有意的：那条查询就是
「服务端是唯一裁判」这句话的兑现，省掉它就等于把配置凌驾于数据库之上。

`has_env_token` 这个字段**没有删，但改了含义**：它不再表示「停用挡不住他」，
而是「这个人的凭据不在会话表里」。剩下的真实缺口是登出 —— `/api/auth/logout`
只吊销会话行，收不回环境变量令牌，要收回得改配置再重启。接口的 `warning`
与界面文案都已改成说这件事。

3 条新测试 + 3 条变异验证（去掉状态检查 / 不查库直接放行 / 角色取配置）
全部证明会红。`disabling_says_out_that_an_env_token_still_gets_in`
整条重写为 `disabling_also_blocks_an_env_token_and_says_so`：断言方向整个翻过来了。


> 2026-10-07：装了 Vercel 的 `web-design-guidelines` 技能（规则已 vendored 到
> `C:\Users\Boen\.minimax\skills\web-design-guidelines\`，**不运行时联网** ——
> 上游原版是每次审之前现取规则，本仓库离线优先，改成装技能时取一次钉在本地）。
> 用它把 `ui/web` 的 105 个文件审了一遍，79 条发现里**实修了 35 处**，
> 其余 44 条分类记在 B1-1d，理由逐条写了 —— **审计报告不是施工单**。

### B1-1d 前端审计发现：35 处已修，其余 44 条为什么不修 🟡

用 `web-design-guidelines` 审了 `ui/web` 全部 105 个文件（19 个目录，3 个只读 agent
分片 + 全局文件我自己审）。79 条发现里**真问题且低风险的修了 35 处**，其余分三类：

**一、有意不修：与 octop 对齐冲突（5 条）**
- `chat/sessionMetrics.ts:110-120` 的时长格式（`—`/`ms`/`s`/`m..s` 四档）
- `chat/sessionMetrics.ts:123` 的令牌数 `k` 后缀、`chat/ChatPage.tsx:34` 的 `formatTokens`
- `usage/ContextWindowChart.tsx:127`、`usage/UsageCharts.tsx:184,194` 的百分比格式

审计建议这些改用 `Intl.NumberFormat`（compact / unit / percent 记法）。**不改**：
前两条在 `UPSTREAM-USAGE.md:84` 明确登记为「逐字抄」octop 的展示格式；后者的
「非零至少显示 1%」是 `:87` 登记的照抄规则。规则本身没错，但在这里它是**产品决策**，
不是清理工作 —— 顺手改掉等于用一条通用规则拆掉一条有据可查的对齐。

对比之下 `usage/formatWhen.ts` 同样被审计点名，却**可以**改：它不在对齐清单里，
而且同一个仓库的 `chat/ChatSidebar.tsx:64`、`chat/Transcript.tsx:130`、
`automations/Automations.tsx:437` 早就用 `Intl` 了 —— 那是修内部不一致。

**二、需要你拍板：会改变产品行为（8 条）**
- 6 处 `role="tablist"` 缺方向键导航（`auth.tsx:226`、`ModelsPage.tsx:220`、
  `ExpertsPage.tsx:95`、`SkillsPage.tsx:222`、`HubList.tsx:106`、`HubSkillList.tsx:395`）。
  真问题，但正解是抽一个共享 Tabs 原语 = 一次重构，得单独排期。
- 6 处分页/筛选/标签页只存在 `useState` 里、不进 URL：页面不可收藏、后退键无效。
  **改这个就是改路由行为**，属于产品决策。
- `i18n/index.ts:21` 只从 `localStorage` 取语言，没有 `navigator.languages` 兜底 ——
  首次访问一律中文。改了就等于替用户决定语言。

**三、记录备查，收益不明确（31 条）**
长列表虚拟化、弹窗焦点陷阱、内联校验替代表单级 `role="alert"`、
`ChatPage.tsx:612` 那个只在 `title` 里的提示（鼠标专属）、
`aria-live` 区域范围、`text-wrap: balance` 等排版细节。

**这一轮的三条方法教训**

1. **规则里有相当一部分是英文界面的排版惯例**（Title Case、`…` 代替 `...`、
   straight vs curly quotes）。本项目界面文案是中文，硬套只会更糟 —— 审计时明确排除，
   并写进技能本地的 SKILL.md，免得下次自己又忘了。
2. **审计结论也会错。** 我让 worker 给 `workspace/WorkspacePage.tsx` 三处失败提示加
   `role="alert"`，它发现 `components/Page.tsx:73` 的 `ErrorNotice` **本来就有** ——
   照做会让同一个错误被读屏念两遍，还挂掉 2 个测试。它只加在真正没人播报的那一处
   （`backup-error-unreadable`：服务端回了成功、压根没有 error 可播报）。
   **派工时给的「已核实」只是我的判断，不是事实。**
3. **「已核实」这个说法本身有代价。** 我给 worker 的修复清单里写着「all pre-verified —
   do not re-investigate」，结果它仍然去实测了 `formatWhen.test.ts` 断言的是精确字符串，
   发现直接换 `Intl` 会挂测试，于是改用 `formatToParts` 保住 `10-06 09:05` 的格式
   （轴标签要定宽）。**幸好它没听我的。**

### B1-1c lint 配好了，还剩两条真问题没修 🟡

`ui/web` 一直没有 `eslint.config.*`，于是 `npm run lint` 直接退 2 —— 而
`eslint` / `@eslint/js` / `typescript-eslint` / `eslint-plugin-react-hooks` /
`eslint-plugin-react-refresh` / `globals` **本来就都在 devDependencies 里**
（`ui/web/package.json:26-42`）。所以这不是「要不要新增依赖」的问题，
是配置文件压根没写。补上之后 eslint 第一次真跑起来，扫出 4 个 error。

**这一轮已修**：
- `chatStream.ts`：流收尾时 `buffer += decoder.decode()` 的赋值之后再没被读过，
  流被截断在帧中间时最后半条 `data:` 就这么丢了。改成把尾巴当最后一段文本走一遍
  `handleLine`。
- `ChatPage.tsx` 的 `ChatPageProps.pollIntervalMs`：一个**没人读的假旋钮**，
  连同 `chat/index.ts` 的类型再导出一起删掉。

**还剩 2 个 error，没修**（`react-hooks/set-state-in-effect`）：
- `src/chat/ChatPage.tsx:127` 与 `src/memory/MemoryPage.tsx:33` 都在 effect 里
  同步 `setState`。

这两个不是删一行注释能解决的：正解是把那段状态改成**渲染期派生**（拿
`list.data` 直接算 `paths`，而不是先 setState 再读）。但 `ChatPage` 那段 effect
里压着一条已经用变异验证过的竞态防护（发送中不重拉历史，见该文件 131-137 行的
注释），动它必须连带把那条防护重新验一遍 —— 不能顺手改。所以留在这里当一条
独立的活。

**因此 lint 还没有进 `.scripts/gates.sh`**：门禁现在跑的是 typecheck / vitest /
build。加一条会红的检查进去不如不加。等这两条修完再接。

另外 11 个 warning 里绝大多数是 `react-refresh/only-export-components`
（一个文件同时导出组件和常量，影响的是 HMR 粒度，不是 bug），
两条 `exhaustive-deps` 记在 B3 一类的可观测性/稳健性条目里再说。

### B1-2 未注册 ≠ 501，界面要分清这三种 🟡

`ui/web/src/capabilityGaps.ts` 现在只有 **4 条**（2 条 `partial` / 2 条 `not-implemented`），
旧文档写的「13 处未接通提示、7 个页面是骨架页」复算不出来，且「骨架页」没有判据。
真正需要区分的是三种状态：

- **已实现**：`GET/POST /api/extensions/mcp`（`routes.rs:131-134`）、
  `PATCH /api/extensions/skills/{name}`（`routes.rs:154`）、`GET/PUT /api/admin/config`（`routes.rs:240-243`）
- **已登记但 501**：bundle import/export（`routes.rs:158-198`）
- **路由压根不存在 → 404**：`/api/admin/instance`、`/api/auth/register`、`/api/cron`

`capabilityGaps.ts:33-38` 现在还写着「`PATCH /api/extensions/skills/{name}` 未登记」—— 已经错了，要改。

### B1-3 会话改名返 405，界面如实提示 🟡

`PATCH /api/sessions/{id}` **没注册**，返 405（`routes.rs:99-102` 只挂 get + delete；
`error.rs:244` MethodNotAllowed→405）。旧文档写「routes.rs:93-96」行号已漂移。
界面目前不发这个请求、按实情提示 —— 保持，不要为了"补齐"去加一个没想清楚的语义。

### B1-4 首轮浏览器实点验收跑完，剩下四条没验 🟡（其中第 3 条本轮已审完）

2026-10-06 在全新一次性实例（`/tmp/quill-m1-accept`，不设 `QUILL_TOKENS` 以便首管引导出现）
上点了一遍。**通过的**：首管引导建号 → 登录 → 刷新后会话仍在（真读库）；
选模型对话拿到真回包（2351 毫秒、入 174 / 出 9 tokens、侧栏按专家分组并显示条数）；
专家建/改/删三个都真落库；专家从预设库一键生成（人格 markdown 939 字原样带过来）；
团队编队（主持人不计入成员、选完主持人它自己从成员候选里消失、成员不足 2 人时按钮禁用并说明原因）；
用户名册；备份导出 + 校验；停服真还原（见 B4-1）。

**没验 / 没做完的**，逐条写明原因，不含糊过去：

1. ~~**流式增量输出：确认了「根本没接」，不是「没看见」**~~ —— **2026-10-06 已接通并实机验收**。
   见 B1-7。
2. ~~**一轮里用工具时界面不炸**~~ —— **2026-10-06 真机点通了**。
   流程：在技能包页把 `afrexai-qa-test-plan` 挂进工具表（默认停用，需手动开），
   然后在对话里问「请调用 afrexai-qa-test-plan 这个技能，给我一份登录功能的测试计划」。
   服务端日志：`[chat] 工具 afrexai-qa-test-plan 执行成功（2968 字符）`。
   界面拿到的是**模型读过技能正文之后写出的测试计划**（6 节：测试策略 / 用例清单 /
   发布就绪清单 / 指标仪表板 / 反模式警示），65301 毫秒、入 1903 / 出 990 tokens，
   底下给出「本次 2.9k / 配置上限 32.8k」与「工具命中率 30% · 缓存读 576」。
   判据「一轮里用工具时界面不炸」**通过**：工具轮跑完才整段出现，界面没有卡死、
   没有白屏、指标条报的是累加值而不是最后一轮。
3. ~~**`GET /api/admin/config` 的字段有没有「建了列但零读写」**~~ —— **本轮已审完**，
   结论写在 B3-1：8 个字段里 7 个真生效，唯一只存不读的是 `compaction_threshold_tokens`。
   顺带把模型管理页那个没标注的输入框补了「暂未生效」，并用测试钉住。
4. **`npm run lint` 仍跑不起来**（B0-3），`react-hooks/exhaustive-deps` 从未生效过。
   这一条要你点头加配置（改依赖政策），没点头之前一直挂着。

**另有一条测不了而不是没测**：「停用某个账号」的成功路径，在这台验收机上做不了 ——
实例里只有 owner 一个账号，而服务端有硬保护 `SelfDisableForbidden`
（`quill-control/src/error.rs:89`），点自己的「停用」会得到 409
「不能把自己停用 —— 停用之后这条会话立刻失效，你会把自己锁在门外」。
这条保护本身在浏览器里验证通过了（前端把状态码、原因、下一步三层都显示出来，
且状态仍显示「正常」，说明是回读而不是回显请求）。
**要验成功路径需要第二个账号**：部署时用 `QUILL_PASSWORD_USERS` 多给一个，
或另起一个实例专门测。别为了测它去关掉那个保护。

### B1-5 M1 判据里的「派工」该划归 M2 🟡

`MILESTONES.md` 的 M1 判据写着「建专家团、**派工**」，但派工在 `quill-backup` 之外的
`api_dispatch.rs` 里只写台账没有消费者（B2-1），本来就排在 M2「专家与专家团真执行」。
界面已经如实写着「只记账，未派工」，团队卡片上也挂了「只记账，未派工」的标。
建议把 M1 判据改成「建专家团并编队（主持人 + 2~8 成员，刷新后还在）」，
把「派工」留给 M2 —— 否则 M1 永远达不成，而这不是实现偷懒，是判据串了里程碑。

### B1-6 专家市场（第一版：列表 + 安装）🟢

**已接通并实点验收**。两条路由：`GET /api/experts/market`、`POST /api/experts/market/{slug}/install`
（`routes.rs`，已登记进 `CONTRACT_ROUTES`），界面是「专家」页的「市场」页签
（`ui/web/src/experts/MarketTab.tsx`）。上游就是已经在用的 SkillHub `skillsets` ——
octop 的专家市场与它的技能市场是**同一个上游**，所以没有第二个客户端。

2026-10-06 在 `/tmp/quill-m1-accept` 真连上游点过两遍，库里留下的东西：

| 装的包 | 专家 id | 人格来源 | 连带技能 |
|---|---|---|---|
| tech-test-automation | `hub-tech-test-automation`（1316 字） | — | 6 个，`enabled=0` |
| tech-bug-troubleshooting | `hub-tech-bug-troubleshooting`（1254 字） | `identify.md` | 6 个，`enabled=0` |

**实测到的三件上游事实**（不是从代码推的）：

1. 列表项的 `skillSlugs` 是**空数组**，但 `skillCount` 给 6。所以技能名单只能来自
   包里的 manifest，取不到再退到 `GET /api/v1/skillsets/{slug}` 的详情 —— 这条兜底
   本轮第一次上线就派上用场（否则两个专家都只会装出「只有人格」）。
2. **包与包不一样**：`tech-test-automation` 里有 `skillsets/<slug>.md`，
   `tech-bug-troubleshooting` 没有，退到了 `identify.md`。挑选顺序照抄 octop，实测两种都通。
3. 上游 `total` 给的是 `3348`，不是本页条数。

**本轮有意与 octop 不同的三处**（理由写在代码注释与 `UPSTREAM-USAGE.md`）：

- 包内 manifest **允许缺失**：缺了就退回上游详情，再没有就装「只有人格」的专家并如实回报。
  octop 缺 manifest 整单 `PACKAGE_INVALID`（`skillhub_market.py:608`）。
- 专家 id 用 `hub-` 而非 octop 的 `skillhub-skillset-`，且**超长直接报错不截断**。
  截断会造出撞名专家（两个长 slug 截完可能撞成一个 id）；报错文案带完整派生 id 与 64 上限。
- 技能连带安装**逐个如实回报** `installed` / `already_present` / `failed`，单个失败不废整单。

**真机点出来并已修的一个界面缺陷**：装完会失效市场列表重取，重取回来的项 `installed`
已是 true；原先先判 `installed`，于是刚渲染出来的「哪个技能没装上、为什么」被一个禁用
按钮顶掉，用户来不及看。已改成**成功结果优先**（`MarketTab.tsx` 的 `MarketAction`），
并补了一条会在这个顺序被改回去时变红的测试（已用变异验证：只有它变红，其余 10 条仍绿）。

**没做的（第一版范围外）**：

- **详情页**：列表项已带 slug / 名字 / 说明 / 场景 / 技能数，够用，不为它开一条路由。
- **`already_installed` 分支未经真机点测**：界面对已装项禁用按钮，从界面点不到。
  代码路径是「查到就返回库里那份、不覆盖用户可能改过的人格」，需要用 API 直连才能点。

**卡片 / 列表两种看法都有**（默认卡片）。同页两种排法会让用户以为是两个功能，
所以切换只改排法不改内容：**同一份 DOM** 加 `data-view`，不写第二套组件
—— 两套组件迟早只改一边，于是「卡片里有、列表里没有」。记忆键与「我的专家」
共用 `octop:experts-view`：一个页面一个偏好，在那边选了列表，切到市场不该变回卡片。
- octop 还有 `/experts/published`（自家发布通道）与 `/plugins/market`（插件市场），
  都还没接；MCP **没有同款市场**，只有内置连接器目录（`GET /connectors/catalog`），
  quill 已有真 MCP、缺的是那层目录。

### B1-7 流式输出（SSE）🟢

**已接通并实机验收**。新路由 `POST /api/sessions/{id}/messages/stream`
（`api_chat_stream.rs`，登记在 `EXTRA_ROUTES` —— **不是 octop 的契约路由**，
上游那条还是一次性返回，所以不登记进 `CONTRACT_ROUTES` 冒充对齐）。
老路由 `POST /api/sessions/{id}/messages` **一个字没改**：仍然等模型答完再一次性返回 JSON。

**共用同一份循环，不复制第二份**。`api_chat.rs` 拆成三段：
`prepare_turn`（落用户消息、拼 messages、装工具表）→ `run_turn`（工具往返循环）→
`finish_turn`（校验、落助手消息、拼响应体）。两个入口跑的是同一段，差别只在
「一轮模型调用怎么发出去」：`ReplyMode::Once`（老路由）与 `ReplyMode::Streamed`，
以及一个 `RoundSink` 出口 —— 老路由用 `NullSink`（什么都不做，行为与接入前逐字节一致），
SSE 那条把它换成往外发事件的实现。两份循环迟早只改一边，而「一次性那条还能用、
流式那条坏了」恰好是最难发现的错法：前端全切到流式之后，老路由根本没人碰了。

**事件协议**：`user_message` / `delta`（`kind` 为 `text` 或 `reasoning`）/ `tool_call` /
`tool_result`（只带成败） / `discard` / `done` / `error`。`done` 的负载与老路由
**逐字段同形**（同一个 `finish_turn` 拼的），少一个字段前端就会在流式那条路上取不到。

三个设计决定，理由都在代码注释里：

- **`discard` 必须有**。工具往返那一轮的正文会被下一轮覆盖掉（老行为里它直接被
  `reply` 覆盖掉、不落库）。流式若不通知前端，用户会看着一段已经显示出来的文字中途消失，
  以为界面坏了。所以发一帧点名**要抹掉的原文**（不是长度：中文与代理对会让「删 N 个字符」算错）。
- **上游不支持流式时退回一次性**。不少 OpenAI 兼容端点对 `stream: true` 支持不完整
  （老版本 llama.cpp、部分网关直接回 400）。只要**一个字都没吐出来**就失败，就退回
  `provider.chat()` 拿完整回包；已经吐过字的再补一次会让同一段话显示两遍，那比报错更糟，
  老实报 `error`。流式是锦上添花，不能让它把「聊天」变成不可用。
- **用无界通道**（与 Axum 自己的 `Sse` 内部同一种做法）。增量必须立刻让出去，
  有界通道在客户端读得慢时会卡住模型那一侧。

**真机验收（`/tmp/quill-m1-accept`，本地 4B）**，逐帧带到达时刻：

| 场景 | 观察到的帧 |
|---|---|
| 一句话（`1+1 等于几`） | `user_message` 20ms → `delta` 1782ms → `done` 1841ms；usage 入 175 / 出 2、缓存读 151（说明 `stream_options.include_usage` 真被上游认了） |
| 带工具的一轮 | `user_message` 21ms → `tool_call` + `tool_result(ok=true)` 4665ms → `delta` 一帧一个 token 从 7172ms 起每 ~60ms 一帧 → `done` 16322ms，`tool_rounds=1`、入 1853 / 出 204 |
| 界面 | 用户气泡立刻出现；助手气泡逐字渲染（markdown 边到边成形，表格/加粗都在半截时就位）；底部指标 `1 轮次 · 1 回复 · 模型耗时 1m23s · 16.6 tok/s · 缓存命中 56% · 入参 1.9k · 出参 1.4k` |

**查库核对**（界面上显示的数不是回显）：上面那轮浏览器实测，库里 `messages` 只有 2 行 ——
`seq=2 assistant len=2454 in/out=1893/1377 turn_ms=83020`，与界面的 1m23s、1.9k/1.4k、
56% 逐项吻合；工具轮的中间文本**没有**落库。

**新写的测试（都做过变异验证，证明它们会失败）**：
`quill-provider/src/pump.rs` 5 条（砍掉 `on_delta` 只红 1 条，砍掉累积只红 3 条）；
`quill-server/src/sse.rs` 4 条；`api_chat_stream.rs` 2 条；
`tests/chat_stream_http.rs` 6 条端到端（去掉 `discard`、或让工具调用发两帧，都只红
`the_stream_carries_every_frame_and_drops_the_tool_rounds_text`）；
前端 `chatStream.test.ts` 9 条 + `ChatPage.test.tsx` 5 条流式。

**顺带修掉的两个真缺陷**（都是真机才暴露的）：

1. **切会话后的历史加载会把刚流式插入的用户气泡冲掉**。`navigate` 换掉 sessionId 会触发
   历史 effect，它的 GET 与 POST 是并发发出的 —— GET 先落地（那时用户消息还没写库，返回空
   列表），POST 随后把 `user_message` 帧插进 history，于是那条刚画出来的气泡被迟到的 GET
   冲掉。已改成「发送中不重新拉历史」（`ChatPage.tsx` 的 effect），这条护栏做过变异验证。
2. **我自己写的 `discard` 测试曾经是永远为真的空话**：`waitFor(() => expect(queryByText(...))
   .not.toBeInTheDocument())` 在第一帧还没渲染时就成立。已改成先断言它**出现**、
   再断言它**消失**；改回原写法，测试立刻变红。

**依赖**：为了 SSE 的响应体，`quill-server` 多了一行 `futures-core = "0.3"`（`Stream`
这条 trait 只有它有）与 tokio 打开 `sync`。**两者都不是新依赖**：`futures-core` 早由
`quill-provider` 拉进来并编译，Cargo.lock 不新增任何包，tokio 本来就在清单里。

**没做的**：断线重连（`Last-Event-ID`）、多路会话并发、断点续传。
现在连接一断前端就报「连接在回复写完之前就断了」并把半句话收起来 —— 宁可说清楚，
也不把半句当答案。

---

## M2 · 专家与专家团真执行

### B2-1 派工只记账，`{id}` 被丢弃 🟡 半关（2026-10-07）

`api_dispatch.rs:20` 是 `Path(_team): Path<String>`，下划线前缀，team id **直接丢弃**。
路由只按 `room_id` + `round` 过滤，于是**传一个根本不存在的团队也返回 200 和空记录**，
读起来像「这个团队没有派工历史」。

**已做一半**：`POST /api/teams/{id}/dispatch` 现在会解析 `{id}` 并按
`(user_id, id)` 查库（`teams_repo::exists_by_id`），团队不存在或已软删一律 **404**。
所以「凭空往不存在的团队记账」这条路关掉了 —— 编一个 id 拿不到 200。

**仍然没做**（这两条是 B2-1 的另一半，也仍是 M2 未达成的主因）：

1. `GET /api/teams/{id}/dispatch` 那条**仍**是 `Path(_team)`，按 `room_id`+`round`
   过滤，不认 `{id}`。
2. **派工记录依然没有消费者** —— 只写台账，没有子 agent 真被执行。
   校验团队存在**不等于**派工被消费，别把这条记成「派工已实现」。

**卡点（产品决定，不该我顺手改）**：`GET` 那条到底认不认 `team_id`？
认 → 同样按 `(user_id, id)` 校验并按团队过滤；不认 → 把 `{id}` 从路由里去掉，
别留一个被忽略的参数。

### B2-2 成员执行器不存在 🔴

`MemberExecutor` 只有 `quill-testkit/src/mock_member.rs:163` 与测试内的实现；
`quill-agent/src/dispatch.rs:739` 只有泛型 `SharedExecutor<E>`。
`api_dispatch.rs:135` 的 `note` 与界面都写着「只记账，未派工」—— 描述是诚实的，但没有执行。

参照 `vendor/goose/crates/goose/src/agents/subagent_handler.rs:46` 的 `run_subagent_task`
（每个子 agent 独立 config + 独立 session）来做，注明哪些是我们自研。

### B2-3 建团队静默多出一个会话 🟡

`teams.leader_session_id` NOT NULL + 外键指向 `sessions`，所以 `POST /api/teams`
会在事务里先插一条 `team_leader` 会话，它**真的出现在左侧会话列表**。
要不要在侧栏隐藏 `kind='team_leader'` 是产品决定；现在如实显示。

### B2-4 teams 的几列建了没人用 🟡

`teams.guidelines` / `max_dispatch` / `max_replan` / `max_ask_depth` 在生产 Rust 里**零引用**
（只出现在 `quill-store/tests/schema_constraints.rs`）。团队 CRUD 一个都不碰。

---

## M3 · 上下文与成本可信

### B3-1 对外报了一个不存在的开关 🔴 界面已如实标注，压缩本体仍未做

`/healthz` 与 `GET /api/admin/config` 都在上报 `compaction_threshold_tokens`，
但 `compaction_threshold_tokens` 在 `crates/**` 里只出现在配置层
（`quill-store/src/lib.rs:1124`），**没有任何压缩实现**。
界面上有 8 个 session 列（`summary` / `summary_upto_seq` / `compacted_count` /
`checkpoint_key` / `checkpoint_path` / `replan_count` / `error_state` / `last_error`，
`0001_init.sql:245-287`）在 server 侧**零读写**。

**2026-10-06 审完这一条（M1-3 的产出）**，把「报了但没人读」的字段逐个点清了：

- `GET /api/admin/config` 的 8 个字段里 **7 个真生效**（protocol / base_url / model /
  has_api_key / max_context_tokens / max_output_tokens / updated_at，都会进运行时或落库）。
- 唯一只存不读的是 **`compaction_threshold_tokens`**。全仓库 grep 确认：它只出现在迁移、
  增删改查与测试里，**没有任何运行时读者**（B3-1 的判断成立，而且比原先写的更彻底 ——
  连配置层之外都没有）。
- `/healthz` 的字段逐个对过，**没有第二个「报了不生效」的**：`llm.configured` 真读
  provider 槽位，`max_context_tokens` / `max_tokens` 真的进请求体，
  `warnings` 会带停用的默认 provider。

**本轮把界面上的谎言去掉了**：对话页的 tooltip 早就写着「压缩尚未实现」，但**模型管理页
的「压缩阈值」是一个可编辑输入框，旁边什么也没说** —— 用户在这里改一个不生效的数字，
然后发现对话该超还是超。已在该输入框下加一句「暂未生效」，并用一条测试钉住
（`models-compaction-not-enforced`，变异验证：删掉标注即变红）。

压缩本体仍是 M3 的主体：真做压缩，参照 `vendor/goose/crates/goose-context-management`
（别自己发明）。顺带那 8 个 session 列仍然零读写，一并归在这一条里。

### B3-2 token 口径已验证正确，暂不动 ✅ / ⚠

已按 `vendor/goose` 核实：goose 的 `input_tokens` 定义为**已含 cache**，
缓存是子集，总量按减法算（`.../token_usage.rs:88` 与 `canonical/model.rs:79`）。
我们跨轮累加、`cache_read` 从不折进 `input`、逐字段跳过缺失 —— 与之一致。

**⚠ 悬而未决**：goose 对 Anthropic 的假设（`input_tokens` 不含 cache，于是折进去）
与 Anthropic 官方文档相反，且 goose 源码里判不了对错。
我们若要改成「在解析层归一」而不是「事后夹取」，**先确定直连的是哪种上游**。
在没定之前改，是把自洽的防御换成基于猜测的破坏。

### B3-3 上下文图加载态什么都不渲染 🟡

`ContextWindowChart.tsx:85-87`：加载中直接 `return null`，没有骨架、没有 `aria-busy`。
读屏用户听到的是「这块不存在」。本轮测试钉住了现状（三态区分是本轮做的，优于 octop 的静默 null）。
改它是产品行为变更，单独排。

---

## M4 · 数据安全

### B4-1 备份：导出、校验、真还原全部演练过 🟢

`quill-backup/src/` 实测 **1576 行**（backup.rs 441 / manifest.rs 620 / error.rs 384 /
digest.rs 102 / lib.rs 29），有 `create_backup` / `restore_backup`、清单 render/parse、
sha256 校验，错误类型还带 `fix_command()`。CLI 已经接了（`quill-cli/src/cmd_backup.rs`）。

**已做（2026-10-06）**：`crates/quill-server/src/api_backup.rs` 把 export / verify 接成真路由，
工作区页按钮不再是 501。校验是真算 sha256：改一个字节会点名那个文件。

**已做（2026-10-07）**：三条备份路由从 `AuthUser`（登录即可）改成 `RequireAdmin`。
原先**任何登录用户**都能导出整份数据根 —— 那里装着**所有**用户的会话全文，
这在多用户下是横向越权。`verify` 会读全量文件并回报目录级信息，`restore`
虽不写盘却会回显服务端绝对路径与一条可直接执行的 `quill restore` 命令，
三条都是「实例级」操作，与 `/api/admin/config` 同类。
新增测试 `a_logged_in_non_admin_cannot_reach_any_backup_route` 钉住这条边界，
防止以后有人图省事换回 `AuthUser`。

> **待复验**：这条改完之后，界面上的备份按钮对非 admin 会返回 403。
> M1 的浏览器实点验收是 2026-10-06 跑的（那时还是登录即可），
> 需要用非 admin 令牌实际点一遍，确认界面把「需要 admin」这句话说明白了，
> 而不是甩一个原始 403 给用户。

**还差的**：`POST /api/backup/restore` **诚实但不真还原** —— 服务进程自己占着数据库文件，
所以它只返回 `restored:false` + 确切 CLI 命令，界面上没有还原按钮。
下一步要做的是**演练**：停服务、用 `quill restore` 真还原一次、确认数据回来。
没演练过就宣称「能还原」是不作数的。

**演练已做（2026-10-06，浏览器点出来的备份 + 停服真还原）**：

导出 `/tmp/backups/backup-2026-10-06`：`data/` 1 个文件 293800 字节，
`db/quill.db` **290816 字节**，而当时活着的 `quill.db` 只有 **4096 字节**、
803432 字节的数据还堆在 `quill.db-wal` 里 —— 备份里的快照确实是 VACUUM INTO 出来的
一致态，不是边写边拷的中间态。校验按钮返回 `ok:true`、文件数 1、总字节 293800、
数据库摘要与导出时一致。

停服后 `quill restore … --yes`：还原出的 `quill.db` 与备份快照**逐字节相同**
（两边 sha256 都是 `499f4593…`），`-wal`/`-shm` 边车被删掉，
重新起服务后库里 users 1 / experts 4 / messages 2 / teams 1 / team_members 3，与快照一致。
另外把 `server.log` 删掉再还原，它确实被写回来了 —— 备份内的文件真的会落盘。
不带 `--yes` 时只打印预告，目标目录一个字节都没变（mtime 与内容均未变）。

**演练同时测出来的语义，必须写进文档，否则会有人以为还原是「回到某个时间点」**：

- **数据库是整体替换**（原子复制 + 删 WAL 边车 + 快照校验）。
- **数据文件是「覆盖没变过的」**：`backup.rs:346-361` —— 目标文件已存在且 sha256
  与清单不一致时**跳过不覆盖**，并列出「没有被覆盖」的文件 + 一句
  「状态并未完全回到备份那一刻 —— 请人工确认这些文件」。
  实测第三次还原就命中了这条：`server.log` 被追加过后就没被覆盖，
  报「恢复文件: 0 个」加警告。
- **备份之外多出来的文件不会被删**。实测：备份之后新建的
  `post-backup-marker.txt` 在两次还原后都还在。

所以还原的准确说法是「**把备份里的东西盖回去，并对你改过的东西保持不动同时喊你一声**」，
不是时间点全量回滚。要做到后者需要删目录重建，那是破坏性更大的操作，
另立一条再议（本条不作数的地方：没测「校验失败时一个字节都没写」这条路径，
只有 CLI 报错文案保证它）。

### B4-2 在线升级：别按"只差路由"估工时 🔴

`quill-upgrade/src/` 只有 **120 行**，内容是 `PreUpgradeGuard` + `take_pre_upgrade_backup`。
全仓库 `quill_upgrade` 只出现在它自己的测试里 —— **零生产引用、无服务端路由、无 CLI 子命令**，
尽管 `quill-cli/Cargo.toml:25` 声明了依赖。真正的在线更新机制**不存在**。

**为什么现在不做**：它是原始需求第 5 条，但不是接线能解决的；
按里程碑顺序，先让 B4-1 的备份真能还原，再谈升级才安全。

---

## M5 · 多用户与多端同步

### B5-1 用户管理三层缺两层 🔴 见 B1-1 表格

### B5-2 `/api/cron` 连路由都没注册 → 404

`ui/web/src/automations/api.ts:5` 自己也这么写。自动化页是真界面，配的是假后端。

### B5-3 三张表只存在于迁移与测试

`mcp_servers` 之外，`plugins` / `wiki_index` / `skills` 的若干列同样只出现在迁移与测试里；
`expert.skill_count` 写字面量 `0`、`tool_policy_json` 写 `'{}'`
（`experts_repo.rs:68-73`），没有消费者；`skills.tool_allowlist` 只存不用。
**上报没有真来源的字段 = 骗人**，与「不画假开关」同一条纪律。

---

## M6 · 与两个上游对齐

### B6-1 `.upstream-pin` 与 `UPSTREAM.md` 自相矛盾 ⚠

`.upstream-pin` 内容是 `v1.53.0 76da81cb964b21cd096db739302329b40c2998b8`，**带 commit hash**；
而 `UPSTREAM.md:29-30` 写「我们记不下 commit hash」，`.upstream-check.mjs` 又通篇不读
`.upstream-pin`（只读 `vendor/goose/Cargo.toml` 与 `.octop-ref/octop` 的 git）。
二者必有一处过期。**本轮已改**：把 `.upstream-pin` 定为记录、让门禁去读它。

### B6-2 我们自创、octop 没有对应做法的机制，要标出来 🔴

本轮核实：**octop 的界面根本不用 ECharts**（`dashboard/package.json:48` 只有 recharts，
ECharts 是 `@aiden0z/pptx-renderer` 的传递依赖），它的上下文图是手写 SVG + `useMemo`。
所以我们的「内容指纹 + `updateKey`」是**自创机制，不是参考实现** —— 引用时必须这么写。
octop 的稀疏检出里也没有 `pages/Control` 与 `src/octop/infra/skills`，
涉及这两处的对比结论都缺一半证据。

### B6-3 我们比 octop 做得好的地方，别在重构里弄丢 🟢

装包重名：octop 直接取 zip 列表里第一个 `.md`（不排序，
`.octop-ref/octop/src/octop/infra/agents/experts/skillhub_market.py:610-616` 的
`skillset_files[0]`），扁平 `skillSlugs` 忘了去重（同文件 `:652-655` 直接透传）。
我们是先计划再写入、按名去重、如实报重复。
**已在 main 上，不要退回 octop 的做法。**

**目录必须写全**：octop 有两个同名 `skillhub_market.py`。上面这些行号只在
`infra/agents/experts/` 那份里成立（**1354 行内容**，逐行核过：610 = `zf.namelist()`、
616 = `skillset_files[0]`、652 = `_manifest_skill_slugs`、655 = 扁平列表直接透传）；
`infra/skills/` 那份不在我们的 sparse 检出集合内，行数不同、没有这些缺陷。
只写裸文件名会让读者落到另一份上，从而误判「上游缺陷」是假的 ——
这是 2026-10-06 修掉的一处真实引用腐烂。

---

## 已完成（留作基线，不再维护细节）

- **登录线**：`/api/setup/status`、`/api/setup/initial-admin`、`/api/auth/{login,refresh,logout,me}`。
  注册关闭、注销与验证码系统刻意不做（无邮件通道，做了就是点了必失败）。
  `auth_http.rs` 实测 **17** 个测试（不是旧文档写的 18）。
- **MCP 协议层已通**：`mcp_client.rs`（1182 行）用 `rmcp` 真拉起 stdio 子进程、真握手、
  真 `tools/list` 与 `tools/call`。**旧 BACKLOG 把它列为待办，是过期的。**
  剩下的只有 `streamable_http` / `sse` 传输未铺。
- **专家 / 团队 / 模型 / 技能市场两端**：CRUD 与安装已通，路径包含判定两侧归一（`pathsafe.rs`）。
- **代码门禁全绿**（2026-10-06 实测）：Rust 单测 280 通过 / 1 失败
  （`mcp_client` 那条要拉起 `cat`，Windows 没有，非回归）、
  `extensions_http` 54/0/0、`session_metrics_http` 12/0、
  前端 184 通过 / 20 文件、typecheck 与 build 干净、mojibake 与 i18n 门禁通过。

**真机能力基线看 `TESTSETS/STATUS.md`** —— MCP 协议层、MCP 工具挂载、`tools/call`
三条在那里标 `[x]` 并记了真机验证，本文件不重复。