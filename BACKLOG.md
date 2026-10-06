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

### B0-3 前端 lint 从来没跑起来 ⚠

`ui/web` 下不存在任何 `eslint.config.*`，`npm run lint` 直接退出 2。
既有问题，非本轮引入。后果是 `react-hooks/exhaustive-deps` 这条规则**从未生效过**，
而 `EChart.tsx:146` / `:155` 的 `useEffect` 依赖正好是它该管的。
本轮补的 20 条测试是目前唯一的自动化安全网（前端 164 → 184）。
**卡点**：加 eslint 配置等于改依赖政策 → 按 `WORKING.md` 归「要问你」那一列。

### B0-4 清理本地噪音 ✅

旧 `.gitignore` 只忽略 `/.wsl-*.sh`，同批产出的 `.wsl-*.py`、`.wsl-commitmsg*.txt`
与各类 `.out`/`.log` 全在 `git status` 里裸奔。已放宽为 `/.wsl-*` 等五条规则，
`git status --porcelain` 从 20+ 条噪音降到 0。

---

## M1 · 单人闭环

### B1-1 20 条 501 桩，其中这些界面有真按钮 🔴

`routes.rs` 里 `not_implemented(` 的桩共 **20 条**（旧文档写 28，实际不是）。
界面上点了必失败的：

| 界面入口 | 打到哪 | 后端有没有能力 |
|---|---|---|
| 实例 / 用户管理页 | `GET/POST /api/users`、`PATCH/DELETE /api/users/{id}` | **三层缺两层**：`quill-control` 只有 `pub(crate)` 的 `list_profiles` / `list_invites` / `insert_invite` / `revoke_user_sessions`，没有对外的 `list_users` / `invite`，也没有路由。旧文档写「只差路由」是错的 |
| 设备页 + 管理页的 MCP 卡片 | `PATCH /api/extensions/mcp/{name}` | `routes.rs:140` 仍是桩；但 `GET/POST /api/extensions/mcp` **是真实现**（旧文档把它也算成桩，错了） |
| 工作区页 | `POST /api/backup/export|verify`、三条 `/api/upgrade/*` | backup 能力齐（见 M4），upgrade 只有 120 行守卫 |

**为什么排 M1**：这些是"真按钮、真失败"，不是假开关，但也不产生价值。
建议顺序：备份接线（M4 的实现已在，最快见效）→ 用户管理三层补齐。

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

---

## M2 · 专家与专家团真执行

### B2-1 派工只记账，`{id}` 被丢弃 🔴

`api_dispatch.rs:20` 是 `Path(_team): Path<String>`，下划线前缀，team id **直接丢弃**。
路由只按 `room_id` + `round` 过滤，于是**传一个根本不存在的团队也返回 200 和空记录**，
读起来像「这个团队没有派工历史」。

**卡点（产品决定，不该我顺手改）**：dispatch 到底认不认 `team_id`？
认 → 解析 slug 并校验团队存在；不认 → 把 `{id}` 从路由里去掉，别留一个被忽略的参数。

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

### B3-1 对外报了一个不存在的开关 🔴

`/healthz` 与 `GET /api/admin/config` 都在上报 `compaction_threshold_tokens`，
但 `compaction_threshold_tokens` 在 `crates/**` 里只出现在配置层
（`quill-store/src/lib.rs:1124`），**没有任何压缩实现**。
界面上有 8 个 session 列（`summary` / `summary_upto_seq` / `compacted_count` /
`checkpoint_key` / `checkpoint_path` / `replan_count` / `error_state` / `last_error`，
`0001_init.sql:245-287`）在 server 侧**零读写**。

这是 M3 的主体：真做压缩，参照 `vendor/goose/crates/goose-context-management`（别自己发明）。

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

### B4-1 备份：导出与校验已接线；「真还原一次」的演练还没做 🟡

`quill-backup/src/` 实测 **1576 行**（backup.rs 441 / manifest.rs 620 / error.rs 384 /
digest.rs 102 / lib.rs 29），有 `create_backup` / `restore_backup`、清单 render/parse、
sha256 校验，错误类型还带 `fix_command()`。CLI 已经接了（`quill-cli/src/cmd_backup.rs`）。

**已做（2026-10-06）**：`crates/quill-server/src/api_backup.rs` 把 export / verify 接成真路由，
工作区页按钮不再是 501。校验是真算 sha256：改一个字节会点名那个文件。

**还差的**：`POST /api/backup/restore` **诚实但不真还原** —— 服务进程自己占着数据库文件，
所以它只返回 `restored:false` + 确切 CLI 命令，界面上没有还原按钮。
下一步要做的是**演练**：停服务、用 `quill restore` 真还原一次、确认数据回来。
没演练过就宣称「能还原」是不作数的。

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