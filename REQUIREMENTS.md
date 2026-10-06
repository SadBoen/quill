> **这份文件是需求的唯一来源，第 8–26 行是用户原话，不要改写。**
> 里程碑与验收判据在 `MILESTONES.md`，操作准则与决策权限在 `WORKING.md`，
> 待办明细在 `BACKLOG.md`，上游基线在 `UPSTREAM.md`。
> 2026-10-06 重构：原先混在本文件后半段的「技术原则 / 交付判据 / 协作纪律 / 禁止 / 起步清单」
> 是执行时补的操作规程，不是需求，已移入 `WORKING.md` —— 混在一起会导致分不清
> 哪条约束来自用户、哪条是执行者自己发明的。现状快照也不再留在本文件，会过期。

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

llama.cpp旧了你就更新成最新版本，也可以换其他运行环境，你自已查嘛，不要什么都我提醒你

---

## 项目与环境的客观事实（不是需求，会过期）

项目：quill —— 个人/家庭/小团队自用的 Agent 平台
代码在 D:\96_CoderWorld\quill（WSL 路径 /mnt/d/96_CoderWorld/quill）

环境事实（已核实，别再猜）：
- Rust 在 WSL2，路径 ~/.cargo/bin，不在默认 PATH，先 export PATH="$HOME/.cargo/bin:$PATH"
- 能用git的命令行窗口就不要用powershell
- 本机有 llama.cpp（Windows 版，D:\00_ProgramFiles\llama-b11146）

2026-10-06 备注：本文件原「起步」清单已过期 —— `/api/auth/login` 不再是 501，
Web 界面与 LLM provider 抽象都已落地。现状看 `BACKLOG.md` 与 `MILESTONES.md`。