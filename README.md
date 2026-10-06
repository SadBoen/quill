# quill

个人 / 家庭 / 小团队自用的 Agent 平台。**以 goose 为 Agent 内核、Rust 为唯一实现语言、octop 为产品壳。**

## 先读哪一份

| 文件 | 回答什么问题 | 谁改 |
|---|---|---|
| [`REQUIREMENTS.md`](REQUIREMENTS.md) | 要造什么 —— **用户原话，改之前先问** | 用户 |
| [`MILESTONES.md`](MILESTONES.md) | 现在到哪了、算不算达成 | 执行者 |
| [`WORKING.md`](WORKING.md) | 怎么干活、哪些事该自己定 | 执行者 |
| [`BACKLOG.md`](BACKLOG.md) | 具体还差什么，按里程碑分组 | 执行者 |
| [`UPSTREAM.md`](UPSTREAM.md) | 跟的是 goose / octop 的哪一版 | 执行者 |
| [`UPSTREAM-USAGE.md`](UPSTREAM-USAGE.md) | 我们用到上游的每一个机制，逐条给出处与状态 | 执行者 |

当前进度见 `MILESTONES.md` 的「当前进度」一节。

> **另一条基线**：真实任务测试集（MCP-Atlas + SkillsBench）的进度、逐条结果与
> 「哪些能力已真机验过」，记在 [`TESTSETS/STATUS.md`](TESTSETS/STATUS.md)，
> 出处与许可见 [`TESTSETS/README.md`](TESTSETS/README.md)。
> 它是**跑出来的**事实，`BACKLOG.md` 是**读代码**得出的一致性判断，两者冲突时以 STATUS 为准。

## 验证项目是活的

```bash
bash .scripts/gates.sh
```

一条命令跑完：构建、Rust 全部测试、前端 typecheck / lint / 单测 / 构建、四道文本门禁
（乱码 / i18n / 库资产 / 上游基线），以及三道「门禁自己的自测」。
退出 0 才算活。

只想跑文本门禁（快，Windows 原生 shell 也能跑）：

```bash
node .mojibake-check.mjs    # 用户可见文本里不许有 U+FFFD
node .i18n-check.mjs        # t() 的占位符要与语言包一致
node .library-check.mjs     # vendor 进来的库逐字一致
node .upstream-check.mjs    # 记录的基线与实际一致（--online 会判上游是否已前进）
node .provenance-check.mjs  # UPSTREAM-USAGE.md 里每条上游引用逐行核过
```

## 想知道「现在到底什么状态」

```bash
node scripts/status.mjs          # 全量：每条待办的判据当场跑一遍
node scripts/status.mjs --quick  # 秒级：只跑便宜的判据
node scripts/status.mjs --json   # 机器可读
```

**不要从文档里读状态。** 文档里没有状态 —— 那是手写的，而手写的事实必然腐烂。
要状态就跑上面这条命令：它只跑一下看结果，不读任何手工维护的标记。

输出里「需人工」的那些判据**没有被机器检查过**，它们既不是通过也不是失败，
是一个公开的缺口。详见 [`docs/adr/0004`](docs/adr/0004)。

**已知的环境性失败**：`mcp_client::tests::a_process_that_never_answers_the_handshake_times_out_with_a_runnable_hint`
在 Windows 上失败，因为它要拉起 `cat`，而 Windows 没有这个命令。这不是回归。

## 环境

Rust 在 WSL2（`~/.cargo/bin` 不在默认 PATH，先 `export PATH="$HOME/.cargo/bin:$PATH"`）。
参考源码 `vendor/goose` 与 `.octop-ref/octop` **不入库**，首次 clone 后按 `UPSTREAM.md`
记录的 pin 用 `bash .scripts/fetch-vendor.sh` 拉回来。