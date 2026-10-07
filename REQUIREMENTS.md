> **下面「用户原话」一节是需求的来源，不要改写。**
> **约束的唯一来源是 [`最高指示.md`](最高指示.md)（五条）—— 那份优先于本文件。**
> 里程碑与验收判据在 `MILESTONES.md`，怎么干活在 `WORKING.md`，
> 待办「为什么」在 `BACKLOG.md`，上游基线在 `UPSTREAM.md`。
> **代码事实**在 [`docs/CODE-TRUTH.md`](docs/CODE-TRUTH.md) ——
> 这里不写任何会过期的状态或环境快照。

---

## 用户原话（唯一可信的需求，不要改写）

我要做一个智能体应用，请帮我从需求拆解到架构设计、技术选型、开发排期和验收标准，输出完整交付方案。
“我想打造一款属于个人、团队使用的Agent智能体。以Rust语言为主，以goose项目为核心Agent，将octop或其他Agent上比较优秀的功能迁移过来。其中，资料库采用LLM-WIKI的变种，即xu-wiki（我自已的github）项目（请用rust重写）。
使用github管理、发布，需要有windows, macos, linux的桌面版，和可以在linux服务器上运行的WEB版本。
初步要求：
1. 多用户；
2. 专家与专家团
3. octop与远程octop协作功能
4. MCP, SKILL，插件等多端同步
5. 在线升级
6. 备份
这些功能很多都是goose或octop已经实现了的”
项目请存放在 D:\96_CoderWorld\quill
测试需要的provider，可以从HF上下载个合适本机的模型（文件小，适合Agent，尽量新，还能做RAG量化用，最后这个不是必要条件），搭建本地provider。
WSL可以提供linux编译环境，也可以直接在wsl里面写程序，看你们自已安排。

MVP状态要有最基本的人机交互界面，如web，如TUI美化过的CLI
MVP未达成之前，不向github提交推送。

> **注（2026-10-08，不改动上面的原话）**：这一条已被用户后续指令取代 ——
> 「阶段性提交一次 github，网络不通就暂时存在本地 git 也可以」。
> 见 [`最高指示.md`](最高指示.md) 作废清单。

llama.cpp旧了你就更新成最新版本，也可以换其他运行环境，你自已查嘛，不要什么都我提醒你

---

## 项目定位

quill —— 个人 / 家庭 / 小团队自用的 Agent 平台。

**约束与定位见 [`最高指示.md`](最高指示.md)（唯一来源，五条）。**
本文件不再复述那五条，以免与之漂移。

代码在 `D:\96_CoderWorld\quill`（WSL 路径 `/mnt/d/96_CoderWorld/quill`）。

**环境与代码事实一律不写在这里**（会过期）——见 [`docs/CODE-TRUTH.md`](docs/CODE-TRUTH.md)。
任务拆解、待办「为什么」见 [`BACKLOG.md`](BACKLOG.md)，
「现在到哪了」跑 `node scripts/status.mjs`。