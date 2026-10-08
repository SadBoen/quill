# quill

个人 / 家庭 / 小团队自用的 Agent 平台。

> **约束的唯一来源是 [`最高指示.md`](最高指示.md)（五条）。**
> 2026-10-08 起，除这五条外，以前所有约束一律作废。
> 其余文档只能从最高指示推导；事实一律以代码为准。

## 先读哪一份

| 文件 | 回答什么 |
|---|---|
| [`最高指示.md`](最高指示.md) | **唯一的约束来源**：五条指示 + 推出的规程 + 作废清单 |
| [`docs/CODE-TRUTH.md`](docs/CODE-TRUTH.md) | **以代码为准的事实总账**（每条都带复现命令） |
| [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) | 架构现状、目标分层、防冗余的模块归属表、优先级路线图 |
| [`docs/OCTOP-MIGRATION-INVENTORY.md`](docs/OCTOP-MIGRATION-INVENTORY.md) | octop 还有哪些能力没迁过来（470 条端点逐条，配套 `docs/octop-endpoints.csv`） |
| [`REQUIREMENTS.md`](REQUIREMENTS.md) | 用户原话（原始需求） |
| [`WORKING.md`](WORKING.md) | 怎么干活（命令 / 格式 / 流程） |
| [`MILESTONES.md`](MILESTONES.md) | 目标与验收判据 |
| [`BACKLOG.md`](BACKLOG.md) | 还差什么（为什么，不讲状态） |
| [`UPSTREAM.md`](UPSTREAM.md) | 跟的 goose / octop 是哪一版 |

## 验证项目是活的

```bash
bash .scripts/gates.sh
```

一条命令跑完：构建、Rust 全部测试、前端 typecheck / lint / 单测 / 构建、
四道文本门禁（乱码 / i18n / 库资产 / 上游基线）。退出 0 才算活。

## 想知道「现在到底什么状态」

**不要从文档里读状态。** 要状态就跑命令 —— 它只跑一下看结果，不读任何手工维护的标记：

```bash
node scripts/status.mjs          # 全量：每条待办的判据当场跑一遍
node scripts/status.mjs --quick  # 秒级：只跑便宜的判据
node scripts/status.mjs --json   # 机器可读
```

输出里「需人工」的那些**没有被机器检查过** —— 它既不是通过也不是失败，是一个公开的缺口。

## 环境

Rust 与 Node 都在 WSL2：

```bash
wsl.exe -e bash -lc 'export PATH="$HOME/.cargo/bin:$PATH"; cd /mnt/d/96_CoderWorld/quill && bash .scripts/gates.sh'
```

参考源码 `vendor/goose` 与 `.octop-ref/octop` **不入库**，首次 clone 后按 `UPSTREAM.md`
记录的 pin 用 `bash .scripts/fetch-vendor.sh` 拉回来。

> 实测工具链（2026-10-08）：cargo/rustc `1.99.0`、node `v24.21.0`。
> 工具链**已固定**在 `rust-toolchain.toml`（Q001）；`cargo fmt --check` 与
> `cargo clippy -D warnings` 已在 `.github/workflows/gates.yml` 与 CI 里（Q004/Q005）。
> **改这两句之前先跑一遍命令** —— README 曾在这里写了「没有固定工具链文件、
> gates.sh 不含 fmt/clippy」，两条都过期了很久。

## 已知的环境性失败

集中登记**因为环境而不是因为代码**的失败。看到这些不等于回归；**在 WSL 里跑一遍**
才是判据。每条都附复现命令与 2026-10-08 的实测结果。

| 现象 | 为什么 | 复现 / 实测 |
|---|---|---|
| `mcp_client::tests::a_process_that_never_answers_the_handshake_times_out_with_a_runnable_hint` 在 Windows 上失败 | 该用例要拉起 `cat`，Windows 没有这个命令（另有几条用 `true`） | Windows 侧 `cargo test -p quill-core`；**未验证**：本机没有 Windows 侧 Rust 工具链（cargo 只在 WSL 里跑），无法实跑；此条沿用旧 README 的说法并标注未验证 |
| `node scripts/status.mjs` 在 Windows 上报 B0-1 / B0-2 / B0-3 / B0-6 失败 | 这几条判据用 POSIX 口径（`bash`、`test $(...) -eq 0`、`npx` 的输出路径 `/tmp`）；Windows 侧 `bash`/`test`/`/tmp` 语义不同 | `node scripts/status.mjs` → 报「命令失败（退出码 1）」；同一批命令在 WSL 里 `bash .scripts/gate-selftest.sh` 退出 **0**（实测） |
| status.mjs 报「读不到 vitest 的 json 输出（ENOENT: … D:\tmp\quill-status-vitest.json）」→ 前端侧判据不可信 | 它把 `/tmp/...` 当路径，Windows 侧被解析成 `D:\tmp\...` | 同上；**这是环境问题，不是前端挂了** —— 前端门禁请在 WSL 或 CI 里跑 |
| 前端 `node_modules` 在 WSL 与 Windows 之间共用 | 原生 binding 按平台编译，跑完一侧另一侧会报缺失 | CI 每个 job 全新 `npm ci` 没有这个问题；本地换侧跑之前重装 |

