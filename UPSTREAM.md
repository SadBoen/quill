# UPSTREAM —— 跟随 goose 与 octop 的版本基线

> **重建说明**：本文件曾由上一批人维护。现以代码为准重写：下面的版本号**打开文件读出来的**，
> 不是抄来的。核对日期 2026-10-08。

quill 抄两个上游：一个是 **Agent 内核底座**（goose），一个是 **产品设计基准**（octop）。
它们各自独立升级。**要跟随时先比版本号，只有版本变了才去看差异。**

---

## goose —— Agent 内核底座

| 项 | 值 |
|---|---|
| 仓库 | https://github.com/aaif-goose/goose |
| **我们跟的版本** | **v1.53.0** |
| commit | `76da81cb964b21cd096db739302329b40c2998b8` |
| 版本来源 | `vendor/goose/Cargo.toml:11` 的 `version = "1.53.0"` |
| 机器可读 pin | `.upstream-pin`（格式 `vX.Y.Z <40位sha1>`） |
| 许可 | Apache-2.0 |
| 本地位置 | `vendor/goose/`（**参考源码，不是依赖**；gitignore，不在库里） |

```bash
grep -m3 '^version' vendor/goose/Cargo.toml   # → 1.53.0
cat .upstream-pin                             # → v1.53.0 76da81cb...
```

**取回**：`bash .scripts/fetch-vendor.sh`（fresh clone 上没有 `vendor/`）。

**一致性核对**：`node .upstream-check.mjs`（离线，比对「记录 vs 本地」）；
`node .upstream-check.mjs --online`（联网，落后退出非 0）。

### 我们实际用到 goose 的哪些部分

只有读，没有链接（**没有任何 crate 依赖 goose**）：

| 位置 | 用途 |
|---|---|
| `crates/goose/src/agents/subagent_handler.rs` 的 `run_subagent_task` | 子 agent 独立 config + 独立 session（**quill 尚未接入**） |
| `crates/goose/src/agents/platform_extensions/summon.rs` | `delegate` / `load` 工具、子 agent 调度协议 |
| `documentation/docs/guides/context-engineering/custom-agents.md` | 「自定义 agent = frontmatter + 正文即 instructions」 |
| `crates/goose-context-management` | 上下文压缩（**quill 尚未接入**，见 B3-1） |

---

## octop —— 产品设计基准（只读）

| 项 | 值 |
|---|---|
| 仓库 | https://github.com/TencentCloud/Octop |
| **我们跟的 commit** | `eb28011249c02cafd389b2d424294c6c1b9cf422` |
| **我们跟的版本** | **1.0.2b6**（该 commit 提交信息：`chore: release 1.0.2b6 (#1612)`） |
| 许可 | MIT，Copyright (c) 2026 Octop |
| 本地位置 | `.octop-ref/octop/`（git **sparse checkout**，所以能 diff） |

```bash
git -C .octop-ref/octop rev-parse HEAD        # → eb280112...
git -C .octop-ref/octop log -1 --oneline      # → chore: release 1.0.2b6
```

**sparse 集合**（`git -C .octop-ref/octop sparse-checkout list`）：
`dashboard/src/{api,components,hooks,layouts,locales,pages/Agent,pages/Chat,pages/Experts,routes,utils}` +
`src/octop/{api,infra/agents/experts,infra/agents/persona,infra/db}`。
所以 `src/octop/api/routers`（55 个路由文件、20970 行）**在手边**，可逐行核。

**做真 diff**：
```bash
cd .octop-ref/octop && git fetch --depth 1 origin main
git log --oneline HEAD..FETCH_HEAD
git diff HEAD FETCH_HEAD --stat
```

### 我们从 octop 取了什么

| 取用方式 | 内容 | 一致性 |
|---|---|---|
| **vendor（逐字）** | 预设专家人格 markdown → `ui/web/src/experts/library/` | 逐字，保留 MIT 出处 |
| **只读参考** | 专家页 / 对话页 / 团队页 / 个性化页的结构、文案、交互 | 只对齐结构 |
| **逐字数据** | 16 型 MBTI 档案、28 题题库 → `crates/quill-server/src/mbti/` | 逐字 |
| **移植基准** | `vendor/openoctopus-frontend/` 的 `index.css` | `ui/web/src/index.css` 逐字节一致（sha256 门禁） |

### ⚠ Octop ≠ OpenOctopus

- **Octop** = `github.com/TencentCloud/Octop`（本地 `.octop-ref/octop/`）—— 功能参考只认它。
- **OpenOctopus** = `github.com/Zpoteiti/OpenOctopus` —— **无关项目**，只贡献 `index.css`。

`node .octop-baseline-check.mjs` 全仓扫一遍，把「把 OpenOctopus 当 Octop 功能参考」的引用报红。

---

## 改动这份文件时

只改「版本 / commit」与「核对日期」，改完跑 `node .upstream-check.mjs` 确认两边记录对得上。
本文件只登记**事实**；「我们用某个机制具体接没接」写在 [`docs/CODE-TRUTH.md`](docs/CODE-TRUTH.md) §6。
