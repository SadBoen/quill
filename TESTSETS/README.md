# TESTSETS · 100 条真实测试任务

这 100 条**全部来自公开发布的真实数据集**，不是编的。每条都能在
`tasks.json` 里找到 `source` / `source_id` / `source_url` / `license`，
原始文件留在 `_raw/`（已 gitignore）可逐条回溯。

## 两个来源

| 来源 | 条数 | 许可 | 出处 | 测什么 |
|---|---|---|---|---|
| **MCP-Atlas**（Scale AI） | 50 | CC-BY-4.0 | [HF 数据集](https://huggingface.co/datasets/ScaleAI/MCP-Atlas) · [论文 arXiv:2602.00933](https://arxiv.org/abs/2602.00933) | **MCP 工具调用**：36 个真实 MCP 服务器、220 个工具；每条任务带 `required_tools` 与 `expected_claims`（标准答案断言） |
| **SkillsBench v1.1** | 50 | Apache-2.0 | [HF 数据集](https://huggingface.co/datasets/benchflow/skillsbench) · [论文 arXiv:2602.12670](https://arxiv.org/abs/2602.12670) · [官网](https://www.skillsbench.ai) | **SKILL 使用**：每个任务包自带真实 `environment/skills/*/SKILL.md`，共 87 个任务包 / 232 份技能，本次取前 50 个任务包 / 120 份技能 |

### 筛选口径（`build_tasks.py` 里写死，不是随手抓前 100 条）

- MCP-Atlas：全量 500 条里，**prompt ≥ 80 字、且有 `expected_claims`、且有
  `required_tools`** 的共 261 条，取前 50。太短的任务测不出东西；没有断言
  的任务根本判不了对错 —— 那种任务放进测试集就是自欺。
- SkillsBench：50 个任务包里 prompt ≥ 80 字的，全部保留。
- 不足 100 条时**直接报错退出**，不拿凑数的东西补。

## 文件

```
TESTSETS/
  build_tasks.py      合成脚本（幂等，可重跑）
  tasks.json          100 条任务清单
  skills/*.md         120 份真实 SKILL.md，已剥掉 frontmatter
  STATUS.md           进度表（待跑 / 通过 / 失败 / 已知问题）
  ISSUES.md           问题记录：现象 / 根因 / 复现 / 修复 / 回归
  _raw/               原始下载件（gitignore，体积大）
```

`skills/*.md` 是从 `SKILL.md` 剥掉 frontmatter 后的正文 —— 因为 quill 的
`skills` 表本来就存 `name` 与 `description`（来自 frontmatter），正文落盘到
`QUILL_SKILL_DIR/<slug>.md`。也就是说这批文件**就是 quill SKILL 功能的真实输入**，
可以直接灌进去测。

## 怎么跑

服务端在 `http://127.0.0.1:18777`（令牌 `dev-token`）。两条路子：

1. **浏览器模拟真实用户**（主要路子）：`runner.py` 驱动 MCP 工具面板
   逐条发任务，看界面表现，发现问题记进 `ISSUES.md`。
2. **HTTP 契约层**：`crates/quill-server/tests/extensions_http.rs` 覆盖
   存储层的字段往返、拒绝路径与跨用户隔离。

## 边界（说清楚，别误读）

- 这些任务是为**别的** agent 平台设计的，quill 不可能有 36 个 MCP 服务器。
  这里的用法是：**用它们当真实用户场景去打 quill 的界面与对话链路**，
  找的是 quill 自己的问题（契约漂移、状态显示不一致、错误吞掉、
  隔离失效这类），不是刷 MCP-Atlas 的分数。
- `expected_claims` 是 MCP-Atlas 官方标注的标准答案，**只用于判断
  「quill 有没有把话说到位」**，不要求 quill 复现官方轨迹。
- 模型是本机 llama.cpp + Qwen3.5-4B，能力有限。**答不上来不算 quill 的 bug**，
  但「明明调用了工具却把正文吞了」「界面显示了没接通的东西」算。
