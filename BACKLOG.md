# BACKLOG —— 按里程碑分组的待办明细

**事实基线**：`TESTSETS/STATUS.md`（真机跑过的进度）与本文每条后面括号里的 `file:line`。
**归属**：每条都挂在 `MILESTONES.md` 的一个里程碑下；里程碑没列的条目不许进来。
**过期处理**：发现与代码不符就改这里，不要在别处留快照 —— 本文件以前就是重灾区。

> 2026-10-06 重构说明：旧版这份文件**自相矛盾** —— 前半段的审计结论（28 条 501、MCP 协议层待办）
> 被同一文件后半段与 `TESTSETS/STATUS.md` 推翻，却没有删掉，于是照着它排期会把已做完的事重做一遍。
> 本版删掉了所有被推翻的说法，并给每条加上证据位置。

---

## 状态：跑命令看，不要读这里

**这份文件不再维护状态。** 以前每条标题后面挂着一个 emoji 标记，
2026-10-07 复核发现它们已经和代码对不上（详见 `docs/adr/0004`）。

```bash
node scripts/status.mjs          # 全量：每条待办的判据当场跑一遍
node scripts/status.mjs --quick  # 秒级：只跑便宜的判据
node scripts/status.mjs --json   # 机器可读
```

判据声明在 [`project/items.mjs`](project/items.mjs)，那是**唯一**的来源。
本文件只负责讲**为什么**（背景、踩过的坑、被否决的方案）——
「为什么」稳定，适合写散文；「是什么状态」易变，必须算，不能写。

输出里三种结果，含义不许混：

| 结果 | 含义 |
|---|---|
| **已验证** | 判据跑过了 |
| **未通过** | 判据跑了但没过 |
| **需人工** | **没有被机器检查过。** 这既不是通过也不是失败，是一个公开的缺口 |
| **判据本身坏了** | 绑的测试名对不上 —— 那条待办的真实状态是**未知的**，比「未通过」更严重 |

### 判据的四种形态

- `test` —— 某条具名测试必须存在且通过
- `cmd` —— 一条命令必须退出 0
- `absent` —— 某个路径必须**不存在**（用于「我们删掉了它」）
- `manual` —— 只能人工验证。**必须显眼**，不许混进「已通过」

### 关于里程碑阶梯

以前这里写着一句「上一条的判据全绿，才开下一条」。**它不是规则，是愿望** ——
它既无法检查，也不区分「已规划」和「在做」。真实约束是待办之间的依赖，
记在 `project/items.mjs` 的 `blocks` 字段里。里程碑只是分组的标签。

---

## M0 · 可重复的地基

### B0-1 门禁解析曾经恒为「通过」  已修，但要防复发

`.scripts/gate-selftest.sh` 的文件头记着这件事：旧门禁用 `awk -F'[ ;]'` 数失败，
而 `-F'[ ;]'` 把 `"; "` 当两个分隔符、多出一个空字段，失败数其实在 `$7`。
于是 `$6` 恒为空，`failed` 恒等于 0 —— **跑多少个红都印「通过」**。
现版解析与自测逐字锁在一起（`.scripts/gates.sh` 与 `gate-selftest.sh` 同源）。

**残留**：~~`gates.sh` 本身还没在 WSL 侧实跑过~~ —— **2026-10-06 已在 WSL 侧实跑通过，
退出 0**。见 B0-2。

### B0-2 `gates.sh` 需要一个能跑的落点  已通

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

### B0-3 前端 lint 从来没跑起来  已通（2026-10-07）

`ui/web` 下不存在任何 `eslint.config.*`，`npm run lint` 直接退出 2 ——
既有问题，非某一轮引入。后果是 `react-hooks/exhaustive-deps` 这条规则
**从未生效过**，而 `EChart.tsx` 的 `useEffect` 依赖正好是它该管的。

**已解决**：`eslint.config.js` 已入库（依赖本来就在，缺的只是那份配置，
不需要改依赖政策）。lint 已进门禁，当前 error / warning 条数以
`bash .scripts/gates.sh` 输出为准。

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

### B0-4 清理本地噪音

旧 `.gitignore` 只忽略 `/.wsl-*.sh`，同批产出的 `.wsl-*.py`、`.wsl-commitmsg*.txt`
与各类 `.out`/`.log` 全在 `git status` 里裸奔。已放宽为 `/.wsl-*` 等五条规则，
`git status --porcelain` 从 20+ 条噪音降到 0。

### B0-5 三个空壳 crate：建了但没人用  已删（2026-10-07）

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

### B1-1 20 条 501 桩，其中这些界面有真按钮

`routes.rs` 里 `not_implemented(` 的桩共 **20 条**（旧文档写 28，实际不是）。
界面上点了必失败的：

| 界面入口 | 打到哪 | 后端有没有能力 |
|---|---|---|
| 实例 / 用户管理页 | `GET/POST /api/users`、`PATCH/DELETE /api/users/{id}` | **已通一半**（2026-10-06）：`GET` 真读库并支持真分页，`PATCH` 真启停。`POST`/`DELETE` **有意保持 501** —— 账号只来自部署配置，软删除只做了一半。旧的「三层缺两层」说法已过期：`ControlPlane` 的 `list_users`/`set_user_status`/`create_user`/`create_invite` 一直都在，缺的是 HTTP 那层 |
| 设备页 + 管理页的 MCP 卡片 | `PATCH /api/extensions/mcp/{name}` | `routes.rs` 里仍是桩；但 `GET/POST /api/extensions/mcp` **是真实现**（旧文档把它也算成桩，错了） |
| 工作区页 | `POST /api/backup/export|verify`、三条 `/api/upgrade/*` | backup 能力齐（见 M4），upgrade 只有 120 行守卫 |

**为什么排 M1**：这些是"真按钮、真失败"，不是假开关，但也不产生价值。
建议顺序：备份接线（M4 的实现已在，最快见效）→ 用户管理三层补齐。

### B1-1b 停用曾经挡不住 `QUILL_TOKENS`  已修

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

### B1-1d 前端审计发现：35 处已修，其余 44 条为什么不修

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

### B1-1c lint 配好了，还剩两条真问题没修  已关（2026-10-07）

`ui/web` 一直没有 `eslint.config.*`，于是 `npm run lint` 直接退 2 —— 而
`eslint` / `@eslint/js` / `typescript-eslint` / `eslint-plugin-react-hooks` /
`eslint-plugin-react-refresh` / `globals` **本来就都在 devDependencies 里**
（`ui/web/package.json:26-42`）。所以这不是「要不要新增依赖」的问题，
是配置文件压根没写。补上之后 eslint 第一次真跑起来，扫出 4 个 error。

**2026-10-06 已修**：
- `chatStream.ts`：流收尾时 `buffer += decoder.decode()` 的赋值之后再没被读过，
  流被截断在帧中间时最后半条 `data:` 就这么丢了。改成把尾巴当最后一段文本走一遍
  `handleLine`。
- `ChatPage.tsx` 的 `ChatPageProps.pollIntervalMs`：一个**没人读的假旋钮**，
  连同 `chat/index.ts` 的类型再导出一起删掉。

**2026-10-07 已修**（剩下的两条 `react-hooks/set-state-in-effect`）：
- `src/memory/MemoryPage.tsx` —— effect 里的 `setPaths` / `setCurrentPath` 都能从
  `list.data` 算出来，改成 `useMemo` + 纯推导的 `activePath`，effect 整个删掉。
- `src/chat/ChatPage.tsx` —— effect 开头同步 `setHistory(null)` 搬到渲染期判断。
  这条下面压着一条变异验证过的竞态防护（发送中不重拉历史），所以搬的时候
  **连带把归属一起搬**：发送开始时同步 `setHistoryFor(新会话)`。只搬动作不搬归属，
  会在流结束 `sending` 变 false 时把刚流完的那一屏当成「上个会话的残留」擦掉 ——
  这个坑是 `ChatPage.test.tsx` 当场撞出来的，已写进该文件注释。

**因此 lint 已进 `.scripts/gates.sh`（2026-10-07）**。原先它不在门禁里，
于是「门禁退出 0」与「eslint 报 2 个 error」可以同时成立而没人发现 ——
这是「检查压根没接线」，与 B0-1 那条「判定写错了」同一类。
`.scripts/gate-selftest.sh` 补了三个场景（只有 warning / 有 error / eslint 自己炸了），
并已用注入一条真 error 的方式验证过门禁确实会红。

还剩一批 warning，不进退出码：`react-refresh/only-export-components`
（一个文件同时导出组件和常量，影响 HMR 粒度，不是 bug）与 `exhaustive-deps`。
确切条数以 `npx eslint .` 当场输出为准。

### B1-2 未注册 ≠ 501，界面要分清这三种

审计当时 `ui/web/src/capabilityGaps.ts` 里只有 4 条（2 条 `partial` / 2 条 `not-implemented`），
旧文档写的「13 处未接通提示、7 个页面是骨架页」复算不出来，且「骨架页」没有判据。
真正需要区分的是三种状态：

- **已实现**：`GET/POST /api/extensions/mcp`（`routes.rs:131-134`）、
  `PATCH /api/extensions/skills/{name}`（`routes.rs:154`）、`GET/PUT /api/admin/config`（`routes.rs:240-243`）
- **已登记但 501**：bundle import/export（`routes.rs:158-198`）
- **路由压根不存在 → 404**：`/api/admin/instance`、`/api/auth/register`、`/api/cron`

`capabilityGaps.ts:33-38` 现在还写着「`PATCH /api/extensions/skills/{name}` 未登记」—— 已经错了，要改。

### B1-3 会话改名返 405，界面如实提示

`PATCH /api/sessions/{id}` **没注册**，返 405（`routes.rs:99-102` 只挂 get + delete；
`error.rs:244` MethodNotAllowed→405）。旧文档写「routes.rs:93-96」行号已漂移。
界面目前不发这个请求、按实情提示 —— 保持，不要为了"补齐"去加一个没想清楚的语义。

### B1-4 首轮浏览器实点验收跑完，剩下四条没验  （其中第 3 条本轮已审完）

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

### B1-5 M1 判据里的「派工」该划归 M2

`MILESTONES.md` 的 M1 判据写着「建专家团、**派工**」，但派工在 `quill-backup` 之外的
`api_dispatch.rs` 里只写台账没有消费者（B2-1），本来就排在 M2「专家与专家团真执行」。
界面已经如实写着「只记账，未派工」，团队卡片上也挂了「只记账，未派工」的标。
建议把 M1 判据改成「建专家团并编队（主持人 + 2~8 成员，刷新后还在）」，
把「派工」留给 M2 —— 否则 M1 永远达不成，而这不是实现偷懒，是判据串了里程碑。

### B1-6 专家市场（第一版：列表 + 安装）

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

### B1-7 流式输出（SSE）

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

### B2-1 派工只记账，`{id}` 被丢弃  两端都不再丢弃了（2026-10-07）

`api_dispatch.rs` 的 GET 那条曾是 `Path(_team): Path<String>`，下划线前缀，
team id **直接丢弃**。路由只按 `room_id` + `round` 过滤，于是
**传一个根本不存在的团队也返回 200 和空记录**，读起来像「这个团队没有派工历史」。

**2026-10-07 已做**（两端都改了）：

- `POST /api/teams/{id}/dispatch`：解析 `{id}` 后按 `(user_id, id)` 查库
  （`teams_repo::exists_by_id`），不存在或已软删一律 **404**。
- `GET /api/teams/{id}/dispatch`：同样先校验团队存在，不存在返回 **404**；
  存在时响应里回显 `team_id`，并加 `filtered_by: "room_id+round"` 明说
  列表到底按什么过滤 —— 不让调用方误以为它按团队过滤。

`teams` 的主键就是 `(user_id, id)`，只按 id 查等于把别人的团队算成自己的，
所以校验一律带 `user_id`。测试两条各一例，且都断言了 404 里**不出现** `count`。

**仍然没做**（这是 B2-1 剩下的部分，也是 M2 未达成的主因）：

**派工记录依旧没有消费者** —— 只写台账，没有子 agent 真被执行。
校验团队存在**不等于**派工被消费，别把这条记成「派工已实现」。

**卡点（产品决定）**：台账的键是 `(owner, room_id, round)`，不是 `team_id`。
要让 GET 真正按团队过滤，得把 `team_id` 并进键里 —— 那会改写已有台账，
属于产品契约变更（`WORKING.md` 里「必须问你」那一列），所以这次只做到
「不谎报存在性」为止，没有擅自改键。

### B2-2 成员执行器不存在

`MemberExecutor` 只有 `quill-testkit/src/mock_member.rs:163` 与测试内的实现；
`quill-agent/src/dispatch.rs:739` 只有泛型 `SharedExecutor<E>`。
`api_dispatch.rs:135` 的 `note` 与界面都写着「只记账，未派工」—— 描述是诚实的，但没有执行。

参照 `vendor/goose/crates/goose/src/agents/subagent_handler.rs:46` 的 `run_subagent_task`
（每个子 agent 独立 config + 独立 session）来做，注明哪些是我们自研。

### B2-3 建团队静默多出一个会话 —— **2026-10-07 已定：侧栏隐藏，已实现**

`teams.leader_session_id` NOT NULL + 外键指向 `sessions`，所以 `POST /api/teams`
会在事务里先插一条 `team_leader` 会话，它**真的出现在左侧会话列表**。
要不要在侧栏隐藏 `kind='team_leader'` 是产品决定；现在如实显示。

**核实过能不能不建：不能，而且也不该。** `POST /api/dispatch` 强制要
`leader_session_id`，成员还得各带一条 `member_session_id`，
`dispatch_ledger.rs:63` 把两者写进派工账本 —— 那条会话就是「谁在这一轮说了什么、
花了多少 token」的落点。要它不存在只能改表结构，那是产品契约变更，
代价远大于收益。

**产品决定：藏。** 理由是它属于实现细节（满足外键约束的手段），
不是用户建的对话；露在侧栏里用户点进去看到的是空的，反而困惑。

**已实现**：`GET /api/sessions?exclude_kind=` 支持按 kind 过滤（逗号分隔），
侧栏请求带 `exclude_kind=team_leader`。过滤放在后端而不是前端，
是因为那条会话跑起来之后真的有消息，前端藏起来会让侧栏条数和实际数量对不上
——「显示的东西必须有真来源」是这条仓库的纪律。
`/api/usage` 那条**不动**：用量统计页按会话列 token，团队跑的花费本来就该算进去。

**⚠ 这套机制是自创的，引用时不能写成「参考上游」**：
- **goose** 完全没有多智能体这套东西，`team_leader` / `leader_session` 零命中；
- **octop** 有 teams，但**没有「主持人会话」这个概念**。
  `manager.py:913` 用 `is_team_agent(row)` 判断一个 agent 是不是团队本身，
  团队就是**专家的名册**（`drop_member` / `reload`），
  没有「给团队开一条会话当工作区」这层。

配套判据在 `crates/quill-server/tests/session_kind_filter_http.rs`（8 条，
做过变异验证）。**上游没得参考，所以钉的是我们自己的契约。**

变异验证里踩了三个坑，全是「判据自己骗人」而不是代码有问题，记下来：

1. **`.filter(|s| !s.is_empty())` 是个伪变异，判据永远抓不到它。**
   删掉这行之后 `NOT IN ('','team_leader')` 与 `NOT IN ('team_leader')` 结果
   **完全相同** —— `''` 匹配不到任何行（kind 的 CHECK 限死在
   `solo`/`team_leader`/`team_member`），SQLite 视它为「这一项没约束」。
   为了这个变异先后写了两版判据（`,solo`、`,team_leader`），都测不出差别。
   现在它**不在变异清单里**，判据 `empty_segments_do_not_change_the_result`
   也改了名并写明「这条现在证明不了什么，别指望它会红」。
   `filter` 本身保留 —— 防御性的：kind 一旦放宽到允许空串，留着空段就会
   真的滤掉那些会话。
2. **想验「空列表别去拼 `NOT IN ()`」，那个变异也是无效的** ——
   实测 SQLite **接受** `NOT IN ()` 并当成恒真（`.scratch/sq.sh`），
   拼了也不报错。换成真会炸的形态才验得住：SQL 不带占位符却仍绑 N 个参数。
3. **变异脚本自己的替换串写错** → 编译失败 → `cargo test` 报 FAILED 但跑的
   其实是**未变异的代码**，判据被误判成「没抓到」。
   现在每个变异是独立 `.py` 文件，里面 `assert` 匹配上了才写盘。

**另外记一条操作纪律**：变异脚本碰的是**生产文件**。中途把它 stop 掉时，
还原没跑到，`.filter` 那行就留在了 `api_chat.rs` 里 —— 直到下一次 grep 才发现。
现在脚本带 `trap cleanup EXIT INT TERM`，并在结束时做逐字节比对校验。

**判据本身也会骗人，得自己验。** 只测默认形态、全空输入、或者挑一个
「两种实现结果恰好相同」的用例，判据就是摆设。

### B2-5 备份与升级塞在工作区里，实例设置是四个点不动的按钮（2026-10-07 已修）

用户实测发现两处信息架构错，查了 octop 才看清根因。

**错一：备份与升级在工作区页。**
`workspace/api.ts`（当时）第一行注释就写着：「工作区页没有文件接口
（`/api/workspace/*` 全部未登记），改接真实存在的备份路由」——
**工作区自己的功能是空的，拿备份来顶替**。用户看到的是「备份属于工作区」。

参照 octop：它**压根没有工作区页** ——
`{ path: "/workspace", element: <Navigate to="/experts" replace /> }`
（`.octop-ref/octop/dashboard/src/routes/index.tsx:213`）直接把 `/workspace`
重定向到专家页。备份归它的 `/admin/backend`（`AdminStoragePage`，`:229`），
升级归 `/admin/advanced?tab=updates`（`:236,244`），**都在 admin 区**。

**已改**：备份与升级搬到 `/admin/backup`，CSS 从 `workspace.css`
搬成 `admin/backup.css`（类名 `oo-workspace-*` → `admin-backup-*`）。
工作区页撤掉三栏骨架 —— **空骨架比一句话更糟**，一个永远空的文件列表框
看着像「文件加载不出来」，而真相是根本没有这个接口。现在只说一句实话 + 指路。

**错二：「实例设置」四张卡片全是点不动的按钮。**
默认人格 / 外部体检服务 / 配额 / 网络抓取，四张卡的输入框全 `readOnly`、
保存按钮全 `disabled`、占位全写「（未接通）」。**四个点不动的按钮比没有按钮更糟**：
它们看起来是设置入口，用户会一个一个去试，试完只得出「这功能坏了」，
而真相是 `ADMIN_CONFIG_ROUTE` 在 quill 里压根没登记。

按项目纪律（不画没有后端的假开关/假按钮）改成**逐项说明为什么不能改**，
并指出真该走哪条路（服务端环境变量 + `quill doctor`）。
参照 octop：它的 admin 区每一项（`/admin/backend`、`/admin/security`、
`/admin/plugins`、`/admin/advanced`）都是**真能操作的**页面，
没有一个是这样一张只读展示页（`routes/index.tsx:229-237`）。

**顺带**：`AdminTabs` 抽成独立文件。备份页搬进 admin 之后立刻出现了两份
页签定义，两处入口将来加一项必然只改一处 —— 「共享 MCP」就是这么消失的
（`AppShell.tsx:31-35` 有那条注释）。判据钉住「只能有一份定义」。

**MCP 市场暂不做**（用户 2026-10-07 决定）。后端 MCP 只有
`GET/POST /api/extensions/mcp`、`DELETE .../{name}`、`PATCH`（501 桩），
**没有任何市场/注册表路由**。octop 那个 `/connectors` 页不在我们的
sparse 检出里（`sparse-checkout` 只有 `dashboard/src/pages/Chat` 与
`pages/Experts`），所以**它到底有没有目录功能没核到** —— 不能照抄，
要做得自己设计接哪个注册表，那是新增外部依赖。

判据在 `ui/web/src/admin/BackupPlacement.test.tsx`（8 条）。


### B2-4 teams 的几列建了没人用

`teams.guidelines` / `max_dispatch` / `max_replan` / `max_ask_depth` 在生产 Rust 里**零引用**
（只出现在 `quill-store/tests/schema_constraints.rs`）。团队 CRUD 一个都不碰。

**2026-10-07 复核**（只统计 `crates/*/src`，测试与迁移不算）：

| 列 | 生产侧命中 | 结论 |
|---|---|---|
| `guidelines` | 0 | 建了没人读 |
| `max_dispatch` | 0 | 建了没人读 |
| `max_replan` | 0 | 建了没人读 |
| `max_ask_depth` | 0 | 建了没人读 |
| `state` / `state_changed_at` | 有引用 | 团队状态机那一层（与 M2 派工一起做） |
| `room_id` | 有引用 | 派工按房间+轮次过滤，已真在用 |

也就是说：**四个限制类列全是摆设**。界面上没有、接口不报、也没有任何逻辑读它们 ——
比 `sessions` 那种「建了列但零读写」更彻底，因为连上报都没有，不至于骗人。
真要做 M2 的派工限额（每轮最多派几个、最多重规划几次、嵌套多深）时才有消费者。

---

## M3 · 上下文与成本可信

### B3-1 对外报了一个不存在的开关  谎言已去掉，压缩本体仍未做（M3）

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

**2026-10-07 复核：这一条由「阻塞」降为「不阻塞」。** 全仓库 grep 确认缺陷本身仍然存在
（`compaction_threshold_tokens` 只出现在 `llm.rs` / `llm_providers.rs` / `api_admin.rs` /
`routes.rs` 的配置、校验、落库与上报里，**没有任何运行时读者**），但**三处界面
都已经如实标注**了：

- 对话页 tooltip：「压缩尚未实现，超过上限不会自动摘要，需要自己新建会话」
- 模型管理页输入框下：「暂未生效」
- 各有一条测试钉住（`models-compaction-not-enforced` 等）

所以「骗人」这一半已经关掉；**剩下的压缩本体是 M3 的功能，不是修 bug** ——
它要决定摘要什么、存到哪（`sessions.summary` 那批零读写列正是给它的）、
压缩后从哪里续上，属于有产品判断的整块工作，不该顺手塞进一次清理。

压缩本体参照 `vendor/goose/crates/goose-context-management`（别自己发明），
连同那 8 个零读写的 session 列一并归在这一条里。

### B3-2 token 口径已验证正确，暂不动  /

已按 `vendor/goose` 核实：goose 的 `input_tokens` 定义为**已含 cache**，
缓存是子集，总量按减法算（`.../token_usage.rs:88` 与 `canonical/model.rs:79`）。
我们跨轮累加、`cache_read` 从不折进 `input`、逐字段跳过缺失 —— 与之一致。

**⚠ 悬而未决**：goose 对 Anthropic 的假设（`input_tokens` 不含 cache，于是折进去）
与 Anthropic 官方文档相反，且 goose 源码里判不了对错。
我们若要改成「在解析层归一」而不是「事后夹取」，**先确定直连的是哪种上游**。
在没定之前改，是把自洽的防御换成基于猜测的破坏。

### B3-3 上下文图加载态什么都不渲染

`ContextWindowChart.tsx:85-87`：加载中直接 `return null`，没有骨架、没有 `aria-busy`。
读屏用户听到的是「这块不存在」。本轮测试钉住了现状（三态区分是本轮做的，优于 octop 的静默 null）。
改它是产品行为变更，单独排。

---

## M4 · 数据安全

### B4-1 备份：导出、校验、真还原全部演练过

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

### B4-2 在线升级：别按"只差路由"估工时

`quill-upgrade/src/` 只有 **120 行**，内容是 `PreUpgradeGuard` + `take_pre_upgrade_backup`。
全仓库 `quill_upgrade` 只出现在它自己的测试里 —— **零生产引用、无服务端路由、无 CLI 子命令**，
尽管 `quill-cli/Cargo.toml:25` 声明了依赖。真正的在线更新机制**不存在**。

**为什么现在不做**：它是原始需求第 5 条，但不是接线能解决的；
按里程碑顺序，先让 B4-1 的备份真能还原，再谈升级才安全。

---

## M5 · 多用户与多端同步

### B5-1 用户管理三层缺两层  见 B1-1 表格

### B5-2 `/api/cron` 连路由都没注册 → 404

`ui/web/src/automations/api.ts:5` 自己也这么写。自动化页是真界面，配的是假后端。

### B5-3 三张表只存在于迁移与测试

`mcp_servers` 之外，`plugins` / `wiki_index` / `skills` 的若干列同样只出现在迁移与测试里；
`expert.skill_count` 写字面量 `0`、`tool_policy_json` 写 `'{}'`
（`experts_repo.rs:68-73`），没有消费者；`skills.tool_allowlist` 只存不用。
**上报没有真来源的字段 = 骗人**，与「不画假开关」同一条纪律。

**2026-10-07 逐条复核**（只统计 `crates/*/src`，测试与迁移不算 —— 这是关键，
因为这些列大多确实出现在迁移与测试里，只看全仓库会得出「有在用」的错误结论）：

| 表/列 | 生产侧 | 实际形态 |
|---|---|---|
| `plugins` | 仅 `routes.rs`（501 桩） | 建了表、**没有读写**，连上报都没有 |
| `wiki_index` | 仅 `cmd_doctor.rs` | 建了表，doctor 探一下，**没有真实读写** |
| `mcp_servers` | **真在用** | `api_extensions.rs` / `mcp_repo.rs` / `mcp_client.rs` 共 11 处 |
| `sessions.compacted_count` | 0 | 建了列零读写（与 `summary` / `checkpoint_path` 同类） |
| `sessions.checkpoint_path` | 0 | 同上 |
| `skills.tool_allowlist` | 21 处，但全是**存/读/上报** | 存进库、从库里读出来、在 API 里回给前端，**唯独不过滤模型这一轮能调什么** |
| `expert.skill_count` | 5 处 | 恒写 `0`，却照常上报给前端（`api_expert_market.rs:85`）—— 属于「上报了一个不会变的数字」 |
| `expert.tool_policy_json` | 3 处 | 只出现在 SELECT 列清单里，恒写 `'{}'` |

**`mcp_servers` 这一条要更正旧说法**：早期文档写它「只出现在迁移与测试里」，
现在它已经有真实的存储层与调用方了，别再按未接线估工时。

`tool_allowlist` 与 `skill_count` 是这一类里最该处理的两个：前者让用户以为
自己限制了模型的工具、实际没有；后者让界面显示一个永远是 0 的计数。
两条都属于「先摘掉上报，等真做了再挂回去」，而不是留着假装有。

---

## M6 · 与两个上游对齐

### B6-1 `.upstream-pin` 与 `UPSTREAM.md` 自相矛盾

`.upstream-pin` 内容是 `v1.53.0 76da81cb964b21cd096db739302329b40c2998b8`，**带 commit hash**；
而 `UPSTREAM.md:29-30` 写「我们记不下 commit hash」，`.upstream-check.mjs` 又通篇不读
`.upstream-pin`（只读 `vendor/goose/Cargo.toml` 与 `.octop-ref/octop` 的 git）。
二者必有一处过期。**本轮已改**：把 `.upstream-pin` 定为记录、让门禁去读它。

### B6-2 我们自创、octop 没有对应做法的机制，要标出来

本轮核实：**octop 的界面根本不用 ECharts**（`dashboard/package.json:48` 只有 recharts，
ECharts 是 `@aiden0z/pptx-renderer` 的传递依赖），它的上下文图是手写 SVG + `useMemo`。
所以我们的「内容指纹 + `updateKey`」是**自创机制，不是参考实现** —— 引用时必须这么写。
octop 的稀疏检出里也没有 `pages/Control` 与 `src/octop/infra/skills`，
涉及这两处的对比结论都缺一半证据。

### B6-3 我们比 octop 做得好的地方，别在重构里弄丢

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

### B6-4 第三个上游 `vendor/openoctopus-frontend` 的来源查到了：它不是 Octop

**2026-10-08 定位（此前只知道「来源没记」）**

来源是 **`github.com/Zpoteiti/OpenOctopus`**（MIT，Copyright 2026 Yucheng Zou，
`frontend/` 目录，pin `682b22cc22b50f1900fec6212cafb9465a2c2ff3`）。
**与 Octop 没有任何亲缘关系** —— 是一个 21 星的个人版项目，只贡献 `index.css`。

真 Octop 是 **`github.com/TencentCloud/Octop`**（本地检出 `.octop-ref/octop/`）。

**这次事故是怎么发生的**

2026-10-08 实现「微信通道」时，照着 `vendor/openoctopus-frontend/src/channels/api.ts`
写了几百行通道实现。事后核才发现：

- 那棵树整棵搜 `weixin|wechat|iLink|微信|personalization` **零命中**；
- 它的通道只有 `discord` / `dingtalk` 两个适配器（`src/channels/adapters/`）；
- 用户开着的 octop 里那个「个性化」页面（含微信扫码、MBTI、九种通道）属于
  `TencentCloud/Octop` 的 `dashboard/src/pages/Agent/`，在另一个仓库里。

写下来的教训不是「下次核仔细点」，而是**文档里的免责声明会反向授权**：

| 位置 | 原文 | 实际效果 |
|---|---|---|
| `UPSTREAM.md:136` | 「**移植基准** vendor/openoctopus-frontend/」 | 「它有正式身份」 |
| `.vendor-baseline-check.mjs` | 为它建了 sha256 门禁 | 「它被门禁守着，所以可靠」 |
| `UPSTREAM-USAGE.md:111` | 「注意这是第三个项目，不是 Octop」 | 「已经有人尽过责了」 |
| B6-4（本条） | 「它来自哪个仓库全项目没记」 | 「这是个已知待办，不用再查」 |

四条各自看都像尽职，合起来的效果是**没人再去问它凭什么在这儿**。
项目早就知道自己手上多了一个来路不明的第三方，却把它越描越正式。

**修法**

- CSS 基准**保持不动**。`ui/web/src/index.css` 已经与那份文件逐字节一致，
  改指向等于改一个已落地的产物 —— 那是另一件事，不混进这次。
- 新增门禁 `.octop-baseline-check.mjs`：全仓扫一遍，任何把
  `vendor/openoctopus-frontend` 当 Octop 功能参考的引用直接报红。
  允许出现的位置逐条写明白理由（文件级白名单）。
- 通道与个性化页面的参考基准**换成 `.octop-ref/octop/`**。

**这道新门禁自己出过一次「永远绿」的事故**（值得单列，因为它就是本项目
反复吃过的那种亏）：`new URL('.', import.meta.url)` 已经以尾斜杠结尾，
我又拼了个 `'..'`，于是扫的是仓库的**父目录**；同时扩展名集合里存的是
`.md`（带点）而 `slice` 出来的是 `md`（不带点），两边永远对不上，于是
**一个文件都判不成文本**。两个 bug 互相掩盖，结果是：结构完整、有自测、
跑得飞快、输出 OK，**但从没扫过任何文件**。

是靠变异验证抓出来的（注入一处违规引用，门禁照样报绿）。
判据得能证明自己会红 —— 这条老规矩又救了一次场。

**为什么这道门禁停在使用者可见的粒度上**：白名单是文件级的。
第一版想做行级（只放行「解释它是什么」的那几行），但那样白名单会变成一串
行号，任何人插一行就能绕过，改个行号还要改门禁。所以停在文件级：它拦住的是
「把这份 vendor 当功能参考」这个真实错误用法，而那种引用只可能出现在源码与
设计文档里。代价是白名单文件内部不再检查 —— 那几个文件都是说明性的。

### B6-5 通道与个性化页面按 `TencentCloud/Octop` 重做（2026-10-08 起）

起因见 B6-4。用户原话：「通道也搞起来吧，连接一个微信就可以了」
+「通道 + 个性化页面一起做」。

**Octop 侧核到的真结构**（`.octop-ref/octop/`，**只读对齐**）：

| 东西 | 位置 | 要点 |
|---|---|---|
| 通道表 | `src/octop/infra/db/repos/channels.py:11-20` | `channel_id` / `agent_id` / `user_id` / `kind` / `name` / `config_json` / `enabled` |
| 配置类型 | `dashboard/src/api/types/channel.ts:96-106` | 九种：discord/dingtalk/feishu/qq/yuanbao/dashboard/wecom/weixin/octopbot |
| 微信多账号 | 同上 `:70-84` | `accounts[]`，每账号 `account_id/account_name/base_url/token/bot_uin/user_uin` |
| 准入策略 | 同上 `:60` | `dm_policy`：`open` / `allowlist` / `pairing` / `disabled` |
| REST 路由 | `src/octop/api/routers/channels.py:118-241` | `/agents/{agent_id}/channels/{channel_id}` + 各平台 `/qrcode/generate` `/qrcode/poll` |
| 微信扫码 | `src/octop/infra/gateway/channels/qr_bind.py:102-136` | 调 `octop_gateway.channels.weixin.login_qr`，即同一套 iLink 接口 |
| 个性化页面 | `dashboard/src/pages/Agent/Personalization/` | 含 `MBTISelector`、`AgentPluginsPanel`、`EditDrawer` |

**刻意不学的**：Octop 的部分通道靠**拉起 Chrome 自动化**完成绑定
（`_safe_profile_directory` 见 `channels.py:352`、`_pkill_chrome_profile` 见 `:387`）。
本项目没有浏览器依赖，微信走纯 HTTP，不受影响。

#### 「八张人格卡」是个误会，Octop 那边是七页签聚合（2026-10-08 核清）

先前记的「个性化页面有八张人格卡（identity/soul/profile/agents/tools/
bootstrap/heartbeat）」**不成立**。逐行核过真身
（`.octop-ref/octop/dashboard/src/pages/Agent/Personalization/index.tsx:30-49`）：

- 那个页面的页签只有七个：`skills` / `subagents` / `tools` / `plugins` /
  `mbti` / `memory` / `channels`；
- 每一签都是**复用别处的面板**（`SkillsTabs`、`ToolsTabs`、`SubagentManager`、
  `MemoryPanel`、`ChannelsPanel`），不是新写的；
- `IDENTITY.md` / `SOUL.md` / `HEARTBEAT.md` / `BOOTSTRAP.md` / `AGENTS.md` /
  `MEMORY.md` 这些是**专家库里的文件**（全仓分别 9 / 19 / 5 / 3 / … 个），
  不是这个页面的页签，而且 `api/routers/agent_files.py` **不读它们** ——
  那个路由管的是心跳配置与每日记忆。人格文件由 agent 运行时读，不在这条线上。

**quill 这边的结论也纠了一个**：人格注入**是通的**
（`experts.instructions` 一列 → `resolve_persona` → system prompt），
而 `ui/web/src/experts/library/*/SOUL.md` 那批文件是**移植时留下的史料**，
`library.ts` 的文件头写明了「只抽取 preset 元数据 + 风格正文」。
所以「把这些 md 接进 prompt」这件事**不成立** —— 它们本来就不该生效，
接上去反而会让用户改史料以为改了行为。

于是这一批真正该做的是**聚合页**（把六个已有页面索引到一处），
外加两处「不许粉饰」：
- **MBTI 那一签明说没做**。Octop 有（`components/MBTISelector.tsx`），
  但那要一张四维光谱表 + 28 题测评结果的存储，本项目没有。画个点不动的
  选择器比不画更糟。
- **子智能体那张卡写明「派工只记账不执行」**（见 B2-2：`MemberExecutor`
  只有测试里的实现）。这是全页唯一一个「承诺会落空」的地方。

**这一轮又踩了一个判据歧义**：`screen.getByText(/还没做/)` 在给子智能体卡
也写上「还没做」之后命中两处，报错与被测行为无关。**判据自己有歧义，
报错就是假警报** —— 与 M3 那次「没抓到」同一类，只是方向相反。

**还有一个伪变异，值得单独记**：`t('key', { defaultValue: 'X' })` 里
**改 `defaultValue` 不改渲染** —— 只要语言包里有那个 key，界面上就是译文。
于是「把 MBTI 标题的 defaultValue 改掉」这种变异跑出来全绿，看起来像
「判据没覆盖」，其实是**变异本身无效**。

它还牵出一件事：`.i18n-check.mjs` **只校验占位符一致，不校验词条存在**
（见本文件更早处记的盲区）。所以「改了 defaultValue 以为改了界面」
这件事很可能已经发生过而不自知 —— 改一次发现没反应，多半是被翻译盖住了，
不是代码没走到。

**判断一个变异是否有效，问的是「门禁观察到的东西变了吗」**，
不是「我改的代码变了吗」。

**已完成**：迁移 `0009_channels.sql`、`channels/{store,weixin}.rs`、`api_channels.rs`
（REST 线 + 微信扫码三步 + 长轮询后台任务）、`ui/web/src/channels/`（页面 + 判据）。

#### 这一轮踩的三个坑（都是判据骗人，不是代码难写）

**1. cargo 按 mtime 判断是否重编 —— 还原后的文件可能更旧**

变异验证用 `Copy-Item` 还原源码时，**还原出来的 mtime 可能比变异期那次编译还旧**，
于是 cargo 认为「没变过」，直接复用**变异版二进制**。表现是：判据在
**完全正确的源码**上报出一个不存在的失败。

它让本轮骗了我两次：先是 `channels_are_scoped_to_their_owner` 报「B 看到了 A 的通道」，
而后是 `unknown_policy_fails_closed` 在源码明明是 `_ => false` 时挂掉。
两次都靠「加一行代码迫使重编」才现出原形。

**判据脚本现在每次跑之前先 `touch` 被测源文件**，这条不能省。

**2. 多段测试只取最后一段的退出码**

`ch-test.sh` 原来对 lib 单测和 HTTP 判据各跑一次 `cargo test ... | tail -6`，
只检查最后一条命令的退出码。结果 lib 段已经 FAILED，脚本却拿 HTTP 段的 0
当成整体通过 —— **判据自己骗了自己**。改成逐段检查 + 任何一段红就整体红。

**3. 纯函数变异只碰得到 `#[cfg(test)]` 里的单测**

`peer_allowed` 的「未知策略必须 fail-closed」是 lib 单测，只跑
`--test channels_http` 抓不到它。同理 `store` 与 `weixin` 协议层。
所以变异验证必须跑**三段**：lib（channels::）、lib（api_channels::）、HTTP 判据。

**「变异没抓到」先怀疑判据没覆盖到，再怀疑变异无效** —— 本轮 M3 连着两次
「没红」，一次是原因 3，另一次是过期二进制。

**4. 变异脚本的备份/还原会吃掉运行期间对源码的修改**

这是上三条之外的第四个，单独记是因为它**直接改生产代码**：脚本在开头备份，
在 `finally` 里还原。而它要跑十几分钟 —— 这期间我修的两个真 bug（游标抹掉
会话映射、链式索引 panic）就被还原成了旧版，编译通过、判据全绿，毫无异样。

所以规矩是：**变异验证期间不要同时改被验的文件**。要么等它跑完再改，
要么把备份改成「按 git HEAD 还原」而不是按时刻还原。
**未完成**：`api_channels.rs` 的 REST 线与长轮询后台任务、前端 personalization 页。

顺带说明 CSS 基准那侧的情况（**没动**）：`.scripts/fetch-vendor.sh` 只取 goose 与
octop，从来不取它；它没有 `.git`（`git -C vendor/openoctopus-frontend` 会一路往上
找到仓库根，于是看起来「有 git」，其实是纯拷贝 —— `.upstream-check.mjs` 里特意
警告过这个陷阱）。它只存在于一台机器上，谁 clone 下来都拿不到。

「index.css 有 sha256 门禁」这句话原本是假的 —— 那道检查只躺在
`.wsl-css-check.sh` 等四个一次性脚本里，`gates.sh` 与 CI 都没跑。现已搬成正经门禁
`.vendor-baseline-check.mjs`，钉哈希而非比对两个文件，所以 `vendor/` 不在手边也能核。
另外 `.provenance-check.mjs` 原来把「整棵树没取回来」判成「引用坏了」，
现已区分「没核」与「坏了」。

---

### B6-6 人格（MBTI）：28 题测评 + 四维光谱 + 应用到某个专家（2026-10-08）

起因是 B6-5 里的那条「明说没做」。用户要求把 Octop 个性化页面的功能学过来，
MBTI 是其中唯一一块既要新数据结构、又真有后端的，所以单独排一批。

**核到的上游**（`.octop-ref/octop`，pin `eb280112`）：
- 端点：`src/octop/api/routers/mbti.py`（`/types`、`/test/questions`、`/test/submit`、
  `/apply`、`/current`）
- 计分：同文件 `_score_answers`（`:617-673`）
- 题库与档案：`mbti.py:589` 处的 `_QUESTIONS`（28 题）、
  `src/octop/infra/agents/persona/mbti_profiles.py`（16 型，596 行）
- 前端：`dashboard/src/pages/Agent/Personalization/components/MBTISelector.tsx`
  （636 行）、`MBTITest.tsx`（395 行）

**照搬的**：16 型档案的全部字段（含 `nickname_zh` 那种民间绰号）、28 题题干与选项、
计分算法、六个端点的形状。

**我们的三处选择**：
1. **不写 SOUL.md。** Octop 把风格段渲染进工作区的 SOUL.md（`mbti.py:58`），
   本项目没有那条链路 —— 人格正文是 `experts.instructions`（早就通了）。
2. **`/api/mbti/apply` 必须带 `expert_id`。** 本项目的人格挂在**专家**上，
   一个用户有多个专家，没有「当前智能体」这个说得清的默认目标。宁可多选一次，
   也不替用户决定把 INFP 的语气写进谁的嘴里。判据钉了「空串也必须拒」。
3. **存历史，不只存当前类型。** Octop 只有一个 `persona_mbti` 字段（`mbti.py:6-8`），
   本项目每次测评留一行（`0010_mbti.sql`），保留最近 20 条，附原始作答 ——
   只存 code 的话，改题库后没法解释「上次的分是怎么算出来的」。

**照搬时最容易被「顺手改对」的三处**（判据就钉在这三处）：
- 强度百分比**不是**「选 A 的比例」，是 `50 + 占比*35` 再夹到 `[50,85]`。
  全选一边也只有 85%。平手时是 **68**（`50+17.5`），不是 50；
  50 只在某轴一题没答时出现。
- 平手取**先出的那一极**（`first >= second`）。
- 题库 28 题、每轴 7 题；`a_pole`/`b_pole` 必须与轴对上，计分只认这个关系。

**已完成**：`mbti/{mod,profiles,questions,score,store,apply}.rs`、`api_mbti.rs`、
迁移 `0010_mbti.sql`、`ui/web/src/mbti/`（页面 + 判据）、
个性化页的 MBTI 从「还没做」换成真卡片。
判据：lib 21 条 + HTTP 13 条 + 前端 16 条。

**这一轮踩的坑**：
- 搬运工按我给的规则「Python 的 `\u265c` 在 Rust 里原样保留」，结果 Rust 只认
  `\u{265c}`。**任务书里写错的事实会被执行者当成规格** —— 写规则前自己得核一遍。
- 前端选项按钮的无障碍名被字母角标 `A`/`B` 吃掉，判据按选项文本找按钮就找不到。
  角标是视觉标记，标 `aria-hidden` 才对 —— 这不只是为了让判据过。
- 我把 `/api/experts/{slug}` 写成了字面量（忘了插值），判据立刻报 400，
  一眼看出是判据自己写错而不是后端坏。

**这一轮变异验证抓到两条判据歧义，都属于「报错/通过的理由不是我们以为的那个」**：

1. **跨用户那条一开始在测「专家不存在」。** 用例让 B 拿 A 的 `row_id` 去 apply，
   `expert_id` 填了个根本不存在的名字，于是即便后端真把 A 的记录读了出来，
   后面查专家那一步照样报 404，断言照样通过 —— **测的是 404，不是越权**。
   去掉 SQL 里的 owner 条件跑出来全绿才暴露出来。
   已改成：B 先建一个**自己真实存在**的专家，再断言 404，并且补一条
   「B 的专家 instructions 必须还是空串」，万一哪天有人把 404 改成 500 也照样现形。
2. **空 `expert_id` 那条只断言了 400。** 放过空串之后，后面
   `ExpertId::parse("")` 同样会 400，判据照样绿。补上「报错文案里必须点名
   `expert_id`」，才分得出是字段校验还是标识格式校验。
3. **「历史封顶」那条测的是接口的 `LIMIT`，不是存储层的清理。**
   `GET /api/mbti/history` 自己就 `LIMIT 20`，所以**就算 DELETE 那一段
   整个不执行，接口照样返回 20 条**。两个变异（把清理的 `LIMIT ?` 写死成
   9999、把 `.bind(KEEP_RECORDS)` 改成 9999）跑出来都全绿。
   补了一条直接 `SELECT COUNT(*)` 数库里的行数才抓住。

**规律**：断言状态码等于断言「某处报错了」；断言接口返回值等于断言
「这个投影是对的」。一个行为有两处能产生同一个结果时，判据就分不出是哪一个 ——
要么把另一处拆出去，要么**直接去看事情真正发生的地方**（字段校验看文案，
存储清理去看表）。

**一处改错了形态，是照着想象画的**：个性化页第一版做成七张大卡片铺两排，
用户对着 Octop 指出「是放在右上角的一行导航，跟你现在的两排大框框区别太大」。
回去读上游才发现 `index.tsx:33-41` 是七页签一行、`:140` 内容区只显示当前签、
`:92` 标题拼成「个性化 / 技能」，**连页签顺序都跟我排的不一样**。
**画界面形态之前先读上游那个组件** —— 它通常已经把形态写死了，
而「结构大致像」和「形态一致」是两回事。

**内嵌页面要能不画自己的页头**：把 `<MbtiPage />` 塞进个性化页的页签后，
同一个页面上出现了两个标题、两条滚动条。所以给它加了 `embedded` 开关
（不画 PageHeader、不套 `page-scroll`）。组件复用的坑一般不在逻辑上，
在**它以为自己是整页**这个前提上。

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