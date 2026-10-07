# WORKING —— 操作规程

> **约束的唯一来源是 [`最高指示.md`](最高指示.md)。**
> 2026-10-08 起，除最高指示五条外，旧约束一律作废（清单见该文件第四节）。
> 本文件只写**怎么做**（命令、格式、流程），**不新增任何约束**。

---

## 干活前的两条命令

```bash
# 1) 基线：确认没坏
wsl.exe -e bash -lc 'export PATH="$HOME/.cargo/bin:$PATH"; cd /mnt/d/96_CoderWorld/quill && cargo test --workspace'

# 2) 现状：哪些待办真的做完了
node scripts/status.mjs --quick
```

Rust 与 Node 都在 WSL2。本机命令的通用跑法是：
`wsl.exe -e bash -lc 'export PATH="$HOME/.cargo/bin:$PATH"; cd /mnt/d/96_CoderWorld/quill && <cmd>'`。

---

## 一次交付长什么样（依最高指示第 5 条）

1. 从 [`docs/ARCHITECTURE.md §5`](docs/ARCHITECTURE.md) 或 [`project/items.mjs`](project/items.mjs)
   挑 1~2 件。
2. 改代码；**每条改动都补能证明它会失败的测试**（把实现改回坏样子，测试应当变红）。
3. 跑门禁：
   - Rust：`rustfmt --edition 2021 --check <改过的文件>`、`cargo clippy --workspace --all-targets`、
     `cargo test -p <相关 package>`
   - Web：`cd ui/web && npm run typecheck && npm run lint && npx vitest run`
4. 中文提交，**一个提交一个主题**，写清**为什么**（不是「改了什么」）。
5. 阶段性 `git push`；推不动就留本地 git，并在交付里如实写「本地领先 N 个提交」。

---

## 新加一条待办

在 [`project/items.mjs`](project/items.mjs) 加一条，`verify` 声明「怎么算做完」，再跑
`node scripts/status.mjs --self-check` 核**判据本身**有没有坏（绑的测试名不存在、`kind`
不合法这类，比「未通过」更严重：真实状态未知）。

`kind` 只能选 `test` / `cmd` / `absent` / `manual`。选 `manual` 是诚实的选择，
但必须写清人工怎么验，且它会在输出里单列，不许混进「已通过」。

---

## 事实与判据（依最高指示第 5 条）

| | 放哪 | 谁维护 |
|---|---|---|
| **判据**：「怎么算做完」「为什么这么做」 | `MILESTONES.md` / `BACKLOG.md` / `docs/adr/` | 人 |
| **事实**：测试数、通过与否、每个待办的真实状态 | `node scripts/status.mjs` 的输出 | 机器，不许抄 |

因此 `MILESTONES.md` 与 `BACKLOG.md` 里**不许出现**任何测试数字、状态标记、进度快照。
待办的唯一来源是 [`project/items.mjs`](project/items.mjs)；`BACKLOG.md` 只讲为什么。

---

## 上游怎么用（依最高指示第 2 / 3 / 4 条）

goose 是核、octop 是壳，两个都在仓库里（`vendor/goose`、`.octop-ref/octop`）：

- 内核逻辑**照抄 goose**，搬进来时在注释里标注出自哪个文件。
- 外壳参照 octop，在 Rust / TS 里重写。
- **不许**把两者加成依赖；删掉它们后 `cargo build` 必须照常成功。
- 卡住就回去翻，别凭记忆发明。

版本基线以 `.upstream-pin` 与 `vendor/goose/Cargo.toml` 为准（见 [`UPSTREAM.md`](UPSTREAM.md)）。

---

## 提交纪律

- 直接提交 `main`；一个提交一个主题；文件不重叠时可合成一次提交。
- 提交信息用中文，写清**为什么**。
- 提交前确认工作区干净、门禁跑过、提交信息没有乱码（`node .mojibake-check.mjs`）。
- **可以 push**（旧「不自动 push」已作废）；推不动就留本地并说明。

---

## 代码风格

- 代码里不写解释性叙事注释；只留不看就会误解的那几句（文档不受此限）。
- 迁移文件（`crates/quill-store/migrations/*.sql`）提交之后不再改动，要改就新开一条 ——
  一旦被某个库应用过，再改会让它判成漂移，整条迁移链停住。
