> **这份文件是需求的唯一来源，下面「用户原话」一节不要改写。**
> 里程碑与验收判据在 `MILESTONES.md`，操作准则与决策权限在 `WORKING.md`，
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

llama.cpp旧了你就更新成最新版本，也可以换其他运行环境，你自已查嘛，不要什么都我提醒你

---

## 项目定位（最高指示，不是需求）

quill —— 个人 / 家庭 / 小团队自用的 Agent 平台。

**最高指示（五条同级）：第 1–4 条是用户原话；第 5 条是工程管理方案（由用户采纳）。**

1. 以 **Rust** 为实现语言（前端 TS 属壳，不受此限）；
2. 以腾讯 **octop** 为产品外壳；
3. 以 **goose** 为 Agent 内核；
4. **最终产物只打包 quill 一个项目**（2026-10-08 补充）：goose 与 octop 是**参考源**，
   允许把它们的代码**抄进来**，但**不许把整个项目当依赖**加进 `Cargo.toml`。
   唯一例外是**可调用的 API 服务**（进程外服务）—— goose 是 Rust 库、octop 后端是 Python，
   两者都不是这种，所以两者只能抄、不能依赖。删掉 `vendor/goose` 与 `.octop-ref/octop`
   后 `cargo build` 必须照常成功。
5. **工程管理方案：门禁驱动的持续交付**（Gate-Driven Continuous Delivery）。
   本项目是 **Rust 后端 + Web 前端** 的前后端工程，**两侧进同一条流水线、受同一把门禁**：
   - Rust 侧：`cargo build` / `cargo test` / `cargo clippy -D warnings` / `cargo fmt --check`
     / `cargo deny` / `cargo semver-checks`
   - Web 侧：`npm run typecheck` / `npm run lint` / `npx vitest run` / `npm run build`

   理论根：**部署流水线**（Deployment Pipeline，Humble & Farley《Continuous Delivery》）
   ＋ **适应度函数**（Fitness Functions，Ford/Parsons/Kua《Building Evolutionary
   Architectures》）＋ **ADR**（Architecture Decision Records，Nygard）
   ＋ **DORA 四指标**（变更前置时间 / 部署频率 / 变更失败率 / 恢复时长）。
   一句话：**「能不能交付」由门禁回答，不由人说。**

代码在 `D:\96_CoderWorld\quill`（WSL 路径 `/mnt/d/96_CoderWorld/quill`）。

**环境与代码事实一律不写在这里**（会过期）——见 [`docs/CODE-TRUTH.md`](docs/CODE-TRUTH.md)。
任务拆解、待办「为什么」见 [`BACKLOG.md`](BACKLOG.md)，
「现在到哪了」跑 `node scripts/status.mjs`。