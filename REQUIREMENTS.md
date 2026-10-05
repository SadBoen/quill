
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

项目：quill —— 个人/家庭/小团队自用的 Agent 平台
代码在 D:\96_CoderWorld\quill（WSL 路径 /mnt/d/96_CoderWorld/quill）
唯一需求来源：仓库根目录的 REQUIREMENTS.md（用户原始要求）

环境事实（已核实，别再猜）：
- Rust 在 WSL2，路径 ~/.cargo/bin，不在默认 PATH，先 export PATH="$HOME/.cargo/bin:$PATH"
- 能用git的命令行窗口就不要用powershell
- 本机有 llama.cpp（Windows 版，D:\00_ProgramFiles\llama-b11146）


技术原则：
- Rust 是唯一实现语言
- 优先成熟方案。判断标准是"这个轮子是不是已经有人维护好了"——
  sqlite / zstd / sqlx / axum / serde 这类直接用，不要自己写同类实现
- 要用的成熟前端/组件库就把源码抄进仓库（vendor），不要运行时联网取
  （离线是常态，外链失败只会白屏）
- 不引入需要人工维护的常驻中间件（Redis / PG / 容器编排 / 监控栈）
- 不加解释性叙事注释；代码里只留必要的、不看就会误解的那几句

交付判据（少而硬，不许打折扣）：
- 验证由你自己完成：界面自己开浏览器点，服务自己发请求，命令行自己跑
  并贴输出。不要把验证动作推给用户
- 测试全绿 ≠ 能用。判据永远是"人能不能真的用"

协作与项目管理（充分利用 minimax code）：
- 用 TodoWrite 维护看板，一次只推进 1~2 个明确目标；完成一项立刻更新状态
- 并行用 task 派子 agent，文件所有权必须互不重叠（一个 agent 只写它那批文件），
  其余 agent 只读。并发上限 4
- 派工时必须给子 agent：目标、已有事实、要改哪些文件、禁止做什么、验收判据
- 探索用 explore（只读），实现用 worker，验收用 verifier
- verifier 独立复核，不要拿实现方自己的自检当验收
- 拿不准该用现成方案还是自己写时，先查 vendor/goose 里怎么做的（那是本项目的
  底座），别凭印象发明
- 已有 skill 该加载就加载（code-review / mavis 等），别硬扛

禁止：
- 不要在代码里留下"为什么这么写"的长篇推理

起步（先看现状再动手，不要照抄）：
- quill-server 有 /healthz、/api/experts、/api/teams/{id}/dispatch 等路由，
  /api/auth/login 目前是 501
- 项目里没有任何 LLM provider 抽象，接本地模型是第一步
- 没有 Web 界面，quill-cli 能跑
- 先跑 cargo test --workspace 确认基线，再开工