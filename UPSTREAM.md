# UPSTREAM —— 跟随 goose 与 Octop 的版本基线

quill 抄两个上游：一个是**Agent 内核底座**，一个是**产品设计基准**。它们各自独立升级，
这里记录「我们现在跟的是哪一版」，下次要跟随时**先比版本号，只有版本变了才去看差异**。

> 版本基线以本文件为准。`.octop-ref/README.md` 只讲怎么用 `graph.mjs` 查代码，不重复版本信息。

---

## goose —— Agent 内核底座

| 项 | 值 |
|---|---|
| 仓库 | https://github.com/aaif-goose/goose （原 `block/goose`，已改名） |
| **我们跟的版本** | **v1.53.0** |
| 版本来源 | `vendor/goose/Cargo.toml` 的 `[workspace.package] version` |
| 快照时间 | 2026-10-04 |
| 许可 | Apache-2.0 |
| 本地位置 | `vendor/goose/`（**参考源码，不是依赖**） |

**核对日期**：2026-10-06。上游最新 release 是 **v1.53.0**（发布于 2026-10-02），
即我们**就是最新的**，无版本差异。

### `vendor/goose/` 不在仓库里，取回来才能用

`vendor/` 是 gitignore 的，所以**刚 clone 下来时这个目录压根不存在**。
它既不是「纯文件拷贝」，也不是别的形态 —— 就是没有。

要用它先取：

```bash
bash .scripts/fetch-vendor.sh
```

跑完它是一个**正经的 git 检出**（按本文件记的 pin checkout 到那个 commit）。所以：

- ✅ 能做的：比版本号；读 release notes；**对本地快照做文件级 diff**，
  并用下面记的 commit hash 核对这份检出确实等于那一版。
- 代价：一次 clone 下来不小（仓库 238MB，大头是 documentation 的博客图片与
  见面会照片，对编译一行都用不上）。所以不塞进 git，只在需要时按 pin 取。

**所以跟随 goose 的粒度是逐文件对齐**，前提是先跑上面那条取码命令。
不跑的话连版本号都没法比 —— 那时候「我们跟的是哪一版」只是一句没人验证过的话。

机器可读的基线在 `.upstream-pin`（格式：`vX.Y.Z <40位sha1>`），
`node .upstream-check.mjs` 会把它与 `Cargo.toml`、本文件三处对照，
对不上就报错 —— 否则多一份记录就多一处能悄悄漂移的地方。

### 我们实际用到 goose 的哪些部分

只有读，没有链接：

| 位置 | 用途 |
|---|---|
| `crates/goose/src/agents/platform_extensions/summon.rs` | `delegate` / `load` 工具、子 agent 调度协议 |
| `crates/goose/src/agents/subagent_handler.rs` | `run_subagent_task`：每个子 agent 独立 config + 独立 session |
| `documentation/docs/guides/context-engineering/custom-agents.md` | 「自定义 agent = frontmatter + 正文即 instructions」的规范 |

**quill 没有任何 crate 依赖 goose。**（`crates/quill-testkit/fixtures/boundary/` 里有两处
提到 goose，那是依赖边界测试的假 fixture，不是真依赖。）

### 下次怎么查

```bash
# 1. 我们现在是哪版（离线，不联网）
node .upstream-check.mjs

# 2. 上游有没有往前走（联网；落后会**退出码 1**）
node .upstream-check.mjs --online
```

`--online` 现在是能失败的：落后会退出 1，并打印落后几个 release/commit、
新出的 release 列表、compare 链接，以及**上游这批改动里碰到我们依赖的那些文件**。
查不到上游（网络失败、被限流）同样退出 1，且明说「这是查不到，不是已是最新」——
它不会在没查到的情况下印一句 OK。判定逻辑有自测：
`node .scripts/upstream-check-selftest.mjs`（纯合成输入，不联网）。

离线路径（默认，不带 `--online`）只比对「记录 vs 本地」，不依赖网络。

### ⚠️ sparse 集合之外、离线核不到的部分

`.octop-ref/octop` 是 sparse checkout，以下路径**不在手边**，引用它们时无法逐行核对：

- `src/octop/infra/skills/skillhub_market.py`（端点常量在这里，而**不在** experts 那份）
- `src/octop/infra/skills/skillhub_common.py`
- `dashboard/src/pages/Control/**`（唯一用 recharts 的地方）
- `dashboard/src/pages/Agent/Skills/SkillHubTab.tsx`

所以：`skillhub.rs` 里的三条端点是**真机实测**定下来的，不是逐字抄来的；
文件头注释现在也这么写，别再写成「照抄 infra/skills/ 那份、experts 那份是旧的」——
那份不在手边，「哪份是旧的」根本判断不了。

---

## Octop —— 产品设计基准（只读）

| 项 | 值 |
|---|---|
| 仓库 | https://github.com/TencentCloud/Octop |
| **我们跟的 commit** | **`eb28011249c02cafd389b2d424294c6c1b9cf422`** |
| **我们跟的版本** | **1.0.2b6**（该 commit 的提交信息：`chore: release 1.0.2b6 (#1612)`） |
| commit 时间 | 2026-10-05 07:47:33 +0800 |
| 许可 | MIT，Copyright (c) 2026 Octop |
| 本地位置 | `.octop-ref/octop/`（**git sparse checkout**，所以能 diff） |

**核对日期**：2026-10-06。`git ls-remote origin HEAD` 返回同一个 commit，
即我们**就是 main 最新**，无版本差异。

### Octop 这边可以做真 diff

因为是 git 检出，更新后直接看差异：

```bash
cd .octop-ref/octop
git fetch --depth 1 origin main
git log --oneline HEAD..FETCH_HEAD     # 落后了几个 commit
git diff HEAD FETCH_HEAD --stat        # 改了哪些文件
```

看完差异再决定要不要跟着改。**注意 sparse 集合只 checkout 了建图需要的目录**，
`dashboard/src/pages/Experts`、`dashboard/src/pages/Chat`、`dashboard/src/components`、
`dashboard/src/hooks`、`dashboard/src/api`、`dashboard/src/locales`、`dashboard/src/utils`、
`dashboard/src/routes`、`src/octop/infra/agents/experts`——差异统计只覆盖这些目录。

### ⚠️ 线上站和仓库不是一回事

线上 https://oc.sadinsun.top 跑的是 **v1.0.1**，仓库已经到 **1.0.2b6**。
**以仓库源码为准，不要对着运行站抄**——那会落后一版。

### 我们从 Octop 取了什么

| 取用方式 | 内容 | 一致性要求 |
|---|---|---|
| **vendor（逐字）** | 17 个预设专家的人格 markdown → `ui/web/src/experts/library/` | 必须逐字，保留 MIT 出处 |
| **只读参考** | 专家页 / 对话页 / 团队页的结构、文案、交互顺序 | 只对齐结构，**不要求代码一致** |
| **移植基准** | `vendor/openoctopus-frontend/`（**另一个项目** OpenOctopus）的 CSS | `ui/web/src/index.css` 逐字节一致，有 sha256 门禁 |

**Octop ≠ OpenOctopus，两者是无关项目。**

- **Octop** = `github.com/TencentCloud/Octop`，设计参考 + 功能参考，取用它的
  结构、文案与交互。**功能上的问题（某功能怎么做的、接口长什么样）只认它。**
- **OpenOctopus** = `github.com/Zpoteiti/OpenOctopus`（MIT, Copyright 2026 Yucheng Zou），
  只贡献 `index.css` 这一个文件。

**别把后者当前者用。** 2026-10-08 出过真事故：照
`vendor/openoctopus-frontend/src/channels/api.ts` 写了几百行通道实现，
而那棵树里没有微信、没有 personalization，功能全在 Octop 那边。
`node .octop-baseline-check.mjs` 现在会把这种引用报红。

> 附带一条踩过的坑：这道门禁自己第一版是**永远绿的**——
> 根路径多拼了一层 `'..'` 扫到了仓库外面，同时扩展名一边带点一边不带点
> 导致一个文件都判不成文本。两个 bug 互相掩盖，输出看着完全正常。
> 是靠变异验证（注入违规引用看门禁会不会红）抓出来的。

### 人格（MBTI）：数据逐字搬，落地方式是我们自己的

Octop 的 MBTI 拆在三处，本项目跟着搬：

| Octop | 内容 | 本项目 |
|---|---|---|
| `src/octop/infra/agents/persona/mbti_profiles.py` | 16 型档案，596 行 | `crates/quill-server/src/mbti/profiles.rs`（**逐字**） |
| `src/octop/api/routers/mbti.py:589` 起的 `_QUESTIONS` | 28 题题库 | `mbti/questions.rs`（**逐字**） |
| `mbti.py:617-673` 的 `_score_answers` | 计分 | `mbti/score.rs`（**只读对齐**） |
| `mbti.py:58` 的 `_persist_persona` | 写进 agent 的 `SOUL.md` | **不跟**，见下 |

**落地方式不同，这是我们的选择**：本项目没有 SOUL.md 那条链路，人格正文是
`experts.instructions`。所以 `/api/mbti/apply` 强制带 `expert_id` —— 人格挂在
**专家**上，一个用户有多个专家，没有「当前智能体」这个说得清的默认目标。

**档案里的民间绰号（`nickname_zh`，如「紫老头」「尺子姐」）照抄不改**。
它们看着像可以「优化掉」的文案，但那是上游的数据，不是我们的文案。

**计分里最容易被「顺手改对」的一处**：强度百分比不是「选 A 的比例」，
是 `50 + 占比*35` 再夹到 `[50,85]`。平手时是 68 不是 50；50 只在某轴一题没答时
出现。判据就钉在这上面 —— 「看起来不合理的公式」往往是对齐上游的结果，
而不是 bug。

### MCP transport 枚举：参考命名，不是照抄

`0007_mcp_transport_alignment.sql` 把 `mcp_servers.transport` 对齐成前端发的那套值。
**这套枚举不是从 Octop 逐字抄的**：Octop 只有 `'stdio' | 'streamable_http'` 两个值
（`.octop-ref/octop/dashboard/src/api/modules/connectors.ts:128`），
我们多了 `'sse'` 与 `'builtin'`。所以是「参考它的命名，我们自己做的选择」。

这条更正**刻意没有写回那条迁移文件**：凡是应用过 0007 的库都记着那个文件的字节摘要，
就地改它（哪怕只改注释）会让这些库一律判成漂移，整条迁移链就此停住 ——
2026-10-07 就这么把 `0008` 卡住过，用量统计页与聊天页一起报 `no such column`。
事实记在这里，迁移文件保持与当初被应用的字节完全一致。

---

## 改动这份文件时

只改「我们跟的版本/commit」和「核对日期」两栏，其余是方法说明。
改完跑一次 `node .upstream-check.mjs` 确认两边的记录都对得上。
