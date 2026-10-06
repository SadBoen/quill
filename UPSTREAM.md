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
| **移植基准** | `vendor/openoctopus-frontend/`（另一个项目 OpenOctopus）的 CSS | `ui/web/src/index.css` 逐字节一致，有 sha256 门禁 |

**注意**：Octop ≠ OpenOctopus，是两个不同项目。前者是设计参考，后者是 CSS 移植基准。

---

## 改动这份文件时

只改「我们跟的版本/commit」和「核对日期」两栏，其余是方法说明。
改完跑一次 `node .upstream-check.mjs` 确认两边的记录都对得上。
