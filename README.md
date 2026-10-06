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

当前进度见 `MILESTONES.md` 的「当前进度」一节。

## 验证项目是活的

```bash
bash .scripts/gates.sh
```

一条命令跑完：构建、Rust 全部测试、前端 typecheck / 单测 / 构建、四道文本门禁
（乱码 / i18n / 库资产 / 上游基线）。退出 0 才算活。

只想跑文本门禁（快，Windows 原生 shell 也能跑）：

```bash
node .mojibake-check.mjs    # 用户可见文本里不许有 U+FFFD
node .i18n-check.mjs        # t() 的占位符要与语言包一致
node .library-check.mjs     # vendor 进来的库逐字一致
node .upstream-check.mjs    # 记录的基线与实际一致
```

**已知的环境性失败**：`mcp_client::tests::a_process_that_never_answers_the_handshake_times_out_with_a_runnable_hint`
在 Windows 上失败，因为它要拉起 `cat`，而 Windows 没有这个命令。这不是回归。

## 环境

Rust 在 WSL2（`~/.cargo/bin` 不在默认 PATH，先 `export PATH="$HOME/.cargo/bin:$PATH"`）。
参考源码 `vendor/goose` 与 `.octop-ref/octop` **不入库**，首次 clone 后按 `UPSTREAM.md`
记录的 pin 用 `bash .scripts/fetch-vendor.sh` 拉回来。