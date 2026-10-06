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

### ⚠️ 一个必须知道的限制

`vendor/goose/` 是**纯文件拷贝，没有 `.git`**。所以：

- ✅ 能做的：比版本号；读 release notes 知道这版有哪些变化。
- ❌ 做不到：对本地快照做文件级 diff。我们记不下 commit hash，也就无法说清
  「上游这一版具体改了我们本地这个文件的哪一行」。

**所以跟随 goose 的粒度只能是「整版对整版」**，不是逐文件对齐。要更细的粒度，
得把它换成 `git clone` 的检出（会大很多，且引入它的完整历史）。

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
# 1. 我们现在是哪版
grep -A3 '\[workspace.package\]' vendor/goose/Cargo.toml | grep version

# 2. 上游最新是哪版
curl -s https://api.github.com/repos/aaif-goose/goose/releases/latest \
  | node -e "let s='';process.stdin.on('data',d=>s+=d).on('end',()=>{const r=JSON.parse(s);console.log(r.tag_name, r.published_at)})"
```

版本一样 → 不用管。有差 → 去看 https://github.com/aaif-goose/goose/releases 的 release notes，
只关注上面「我们实际用到」那张表里三个文件相关的变化。

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
