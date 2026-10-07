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
> 仓库**没有固定工具链文件**，`gates.sh` 也**不含** `cargo fmt` / `cargo clippy`。

## 已知的环境性失败

`mcp_client::tests::a_process_that_never_answers_the_handshake_times_out_with_a_runnable_hint`
在 Windows 上失败，因为它要拉起 `cat`，而 Windows 没有这个命令。这不是回归。
