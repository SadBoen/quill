# QUEUE —— 执行队列（持久待办，按顺序取）

> **这个文件是「下一件做什么」的队列。** 每轮（人或自动化）从**最上面未完成的**开始取
> 1~3 条做掉、提交、把 `[ ]` 改成 `[x]`。
>
> 与其它文件的分工：
> - [`最高指示.md`](最高指示.md) —— 唯一的约束来源。
> - [`project/items.mjs`](project/items.mjs) —— **里程碑验收判据**（少而硬，机器判定）。
> - **本文件** —— **可执行的细粒度任务**（多而具体，人/自动化按序取）。
> - `docs/ARCHITECTURE.md §5` —— 为什么是这个优先级。
>
> 每条格式：`- [ ] Qxxx · 做什么 · 落在哪 · 怎么算做完`。
> 取一条做一条，**不要一次动一堆**（最高指示第 5 条：小批量是门禁可信的前提）。

---

## A. 门禁与地基（依最高指示第 5 条）

- [x] Q001 · 加 `rust-toolchain.toml` 固定 Rust 版本（现在**没有**，任何人都可能用不同版本编译）· 仓库根 · 文件存在且 `cargo --version` 与之相符
- [x] Q002 · 加 `deny.toml` 做依赖/许可证门禁 · 仓库根 · `cargo deny check` 能跑（或如实说明为何暂不可用）
- [x] Q003 · 定 `rustfmt.toml` 并做**一次性全量格式化**（单独一个提交，免得淹没真实 diff）· 仓库根 · `cargo fmt --check` 退出 0
- [x] Q004 · 把 `cargo fmt --check` 接进 `.github/workflows/gates.yml` · CI · 工作流里有这一步且能过
- [x] Q005 · 把 `cargo clippy --workspace --all-targets -- -D warnings` 接进 CI（现在刻意没跑）· CI · 工作流里有这一步且退出 0
- [x] Q006 · `api_chat` 内联 SQL 收口到 repo：**会话域已完成**（22 → 13 条，收进新模块 `chat_repo`）· `crates/quill-server/src/chat_repo.rs`
- [ ] Q006b · `api_chat` 剩余 **13 条**内联 SQL（messages / 指标 / `prepare_turn` 发送路径）继续收口到 repo · `crates/quill-server/src/` · 该文件内 `sqlx::query` 计数降到 0
- [ ] Q007 · 按 `BACKLOG.md` B0-4 定 `need_str`/`opt_str` 的规范语义并合并 5+3 份 · `crates/quill-server/src/jsonx.rs` · 只剩一份定义，且补了钉住新语义的测试
- [ ] Q008 · `quill-server` 拆分：把 `api_*` 之外的通用件（`error.rs`/`db.rs`/`state.rs`）之外的巨石按域拆 crate 或子模块 · `crates/quill-server/src/` · 单文件上限显著下降且测试全绿
- [ ] Q009 · 给 `quill-adapters` / `quill-domain` / `quill-store` 加「不许依赖上层」的编译期守卫（如文档 + CI 检查）· CI · 违反时 CI 报红
- [ ] Q010 · 用 `cargo-semver-checks` 或等价手段盯公开 API 兼容 · CI · 至少在 `quill-adapters` 上跑起来

## B. 内核 `quill-core`（依最高指示第 3 条：抄 goose）

- [ ] Q011 · 新建 `crates/quill-core`（空壳 + 一行职责说明）· `crates/quill-core/` · `cargo build -p quill-core` 通过
- [ ] Q012 · 把对话循环从 `api_chat.rs` 搬进 `quill-core`（**原样搬运**，行为不变）· `quill-core` · 测试数量不减、全绿
- [ ] Q013 · 把 `tools.rs` 搬进 `quill-core` · 同上 · 同上
- [ ] Q014 · 把 `mcp_client.rs` 搬进 `quill-core` · 同上 · 同上
- [ ] Q015 · 把 provider 组装（`llm_providers.rs`）搬进 `quill-core`，与 `quill-provider` 合并成一套 · 同上 · provider 组装只剩一套
- [ ] Q016 · 每搬一块，在文件头标注「移植自 `vendor/goose/...` 的哪个文件」· `quill-core` · 头部注释有出处
- [ ] Q017 · 照 `vendor/goose/crates/goose-context-management` 实现**上下文压缩**（现在只有配置字段）· `quill-core` · 超阈值真的发生压缩，用量前后连续
- [ ] Q018 · 压缩阈值接进对话路径，并在界面上**如实反映是否生效** · `quill-core` + `ui/web` · 不再是「只存不读」
- [ ] Q019 · 照 goose 实现**记忆**（当前为零）· `quill-core` · 有可验证的读写回路
- [ ] Q020 · 照 `vendor/goose/crates/goose/src/agents/` 补**状态机** · `quill-core` · 对话状态可枚举、可测
- [ ] Q021 · 照 goose 补**快照**（snapshots） · `quill-core` · 能回滚到某一轮
- [ ] Q022 · 照 goose 补**重试**（retry.rs） · `quill-core` · 上游 5xx/超时有退避重试且可测
- [ ] Q023 · 子 agent：`member_executor` 的 `steer`（运行中追加指令） · `quill-core` · 不再返回「尚未实现」
- [ ] Q024 · 子 agent：`member_executor` 的 `abort`（中途取消） · `quill-core` · 同上
- [ ] Q025 · 子 agent：给成员各开**独立会话**（对齐 goose 的「每子 agent 独立 config + session」） · `quill-core` · 成员产出进各自的会话
- [ ] Q026 · 成员执行也记 **token 用量**（现在 `MemberOutcome` 不带 usage） · `quill-core` · 用量页能看到派工消耗
- [ ] Q027 · 照 goose 的 `permission` 对齐权限模型（现在 quill-control 是多租户口径，需比对） · `quill-core` · 差异写清
- [ ] Q028 · 照 goose 的 `slash_commands` 实现斜杠命令（先核 quill 有没有） · `quill-core` · 至少有 `file:line` 出处对齐
- [ ] Q029 · provider 移植覆盖度逐块比对（`quill-provider` vs `goose-providers`） · `docs/` · 出一张覆盖表
- [ ] Q030 · 会话模型对齐 goose 的 `session`（当前 `api_chat` 自研） · `quill-core` · 差异写清

## C. 把 501 桩接成真接口（`routes.rs` 现有 11 处）

- [ ] Q031 · `POST /api/experts/import` 真实现 · `api_experts.rs` · 导入可往返
- [ ] Q032 · `GET /api/experts/export` 真实现 · `api_experts.rs` · 与 import 对称
- [ ] Q033 · `POST /api/wiki/ingest` 真实现（依赖 Q0xx 知识库） · `api_wiki.rs` · 真写入索引
- [ ] Q034 · `POST /api/wiki/query` 真实现 · `api_wiki.rs` · 真返回答案
- [ ] Q035 · `POST /api/wiki/search` 真实现（只读索引，可不依赖 LLM） · `api_wiki.rs` · 真返回命中
- [ ] Q036 · `PATCH /api/extensions/mcp/{name}` 真实现 · `api_extensions.rs` · 改名/改配置可往返
- [x] Q037 · `GET /api/extensions/plugins` —— **刻意保持 501**：前端 `capabilityGaps.ts` 已把它登记为「已登记路由、处理函数未实现」并如实显示给用户；删掉会让那句「已登记」变假话（退化成 404）。已在路由处加注释说明 · `routes.rs`
- [ ] Q038 · `GET /api/upgrade/check` 真实现 · `api_upgrade` · 能报出当前版本与是否有新版
- [ ] Q039 · `POST /api/upgrade/prepare` 真实现（含升级前备份守卫） · 同上 · 真落备份
- [ ] Q040 · `GET /api/upgrade/history` 真实现 · 同上 · 能列出历史
- [x] Q041 · `GET /api/ws` 真实现或如实删掉 —— **选了删**：核过前端 `ui/web/src` 与全部测试都**没人用**它，它是一条假接口（最高指示第 2/5 条：不画没有后端的入口）· `routes.rs`

## D. octop 外壳迁移（依最高指示第 2 条）

- [ ] Q042 · `/api/cron` 有真路由（自动化页现在配的是假后端） · `crates/quill-server/src/` · 页面上的开关真有用
- [ ] Q043 · 团队限制列 `max_dispatch`/`max_replan`/`max_ask_depth`/`guidelines` 不再只存不读 · `quill-agent`/`api_teams.rs` · 生产侧真有引用
- [ ] Q044 · 不再上报没有真来源的字段（`plugins`/`wiki_index`/`skill_count`/`tool_allowlist`） · `api_admin.rs` 等 · 字段有真来源或删掉
- [ ] Q045 · 通道（channels）按 TencentCloud/Octop 重做 · `api_channels.rs` · 有 `file:line` 对齐
- [ ] Q046 · 个性化页面按 octop 重做 · `ui/web/src/personalization` · 同上
- [ ] Q047 · 专家市场（列表 + 安装）接通 · `api_expert_market.rs` · 真能装
- [ ] Q048 · 备份三条路由只允许 admin 的**回归测试**钉住 · `crates/quill-server/tests/` · 非 admin 拿到 403
- [ ] Q049 · `POST /api/backup/restore` 在演练里真还原成功 · `api_backup.rs` · 端到端可验
- [ ] Q050 · 客户端不能指定落盘位置（`../` 与盘符一律拒）的测试 · `api_backup.rs` · 已有则补反向验证
- [ ] Q051 · 会话改名：`PATCH /api/sessions/{id}` 注册（现在 405） · `routes.rs`/`api_chat.rs` · 真改名
- [ ] Q052 · 建团队时静默多出的那个会话，不在侧栏露出来 · `api_teams.rs`/`ui/web` · 侧栏干净
- [ ] Q053 · 换会话时不残留上一会话的话 · `ui/web/src/chat` · 有回归测试
- [ ] Q054 · 前端审计的「为什么不修」逐条给出理由（若仍在） · `ui/web` · 清单可查
- [ ] Q055 · 分页：列表类接口在没有 `limit` 时的默认行为写清并测 · `api_*` · 不无限返回

## E. 知识库 / 资料库（需求里的 xu-wiki）

- [ ] Q056 · 核 `quill-wiki` 与用户 GitHub 上 xu-wiki 的能力差 · `docs/` · 出一张差集表
- [ ] Q057 · wiki 的 `ingest_context` / `query_context` 接上真 LLM 后端 · `quill-wiki` · 端到端可跑
- [ ] Q058 · wiki 索引的增删改查 HTTP 面接通（见 Q033–Q035） · `api_wiki.rs` · 前端能真用
- [ ] Q059 · 资料库页面（`ui/web/src/workspace`）接真接口 · `ui/web` · 没有假按钮
- [ ] Q060 · wiki 文件路径安全（`pathsafe`）的边界测试 · `quill-wiki` · 越界一律拒

## F. 数据安全 / 备份 / 升级

- [ ] Q061 · 备份导出真落盘，摘要与磁盘文件一致（回归） · `quill-backup` · 端到端
- [ ] Q062 · 备份校验不是装饰：改一字节必须失败并点名文件 · `quill-backup` · 有反向验证
- [ ] Q063 · 在线升级本体（现在只有「升级前先备份」守卫） · `quill-upgrade` · 真能升级
- [ ] Q064 · `quill-upgrade` 要么被真调用，要么删（现在声明了却零调用） · `quill-cli`/`quill-server` · 二选一并说明
- [ ] Q065 · 迁移链的快照/漂移检测有测试 · `quill-store` · 改历史迁移会报红
- [ ] Q066 · 数据库打开的并发与锁参数写清并测 · `quill-store` · 文档 + 测试

## G. 前端（Web 壳）

- [ ] Q067 · 逐页核「未注册 / 查无此人 / 未接通」三态显示正确 · `ui/web/src` · `capabilityGaps.ts` 覆盖
- [ ] Q068 · 流式输出（SSE）在界面上逐帧更新（回归） · `ui/web/src/chat` · 有测试
- [ ] Q069 · 上下文图在加载态有渲染（不是空白） · `ui/web` · 有测试
- [ ] Q070 · 用量页的数字来自真实累加（不是最后一轮） · `ui/web/src/usage` · 有测试
- [ ] Q071 · 没有「点了必失败」的按钮：逐入口核一遍 · `ui/web` · 清单可查
- [ ] Q072 · `ui/web` 的 17 条路由逐条核有对应后端 · `ui/web/src/app` · 出对照表
- [ ] Q073 · 前端 typecheck/lint/vitest/build 全进 CI（现状部分已进） · CI · 四项都在
- [ ] Q074 · i18n 门禁覆盖所有面向用户的字符串 · `.i18n-check.mjs` · 漏翻译会报红
- [ ] Q075 · 移动端/桌面端的入口如实标注可用性 · `ui/web` · 不画假入口

## H. 上游对齐与出处（依最高指示第 3/4 条）

- [ ] Q076 · `.upstream-pin` 与 `UPSTREAM.md` 不再自相矛盾 · 仓库根 · 两者一致
- [ ] Q077 · 自创机制在代码/文档里标注「自创」，不包装成抄来的 · 全仓 · 有清单
- [ ] Q078 · 每条上游引用逐行核过（`node .provenance-check.mjs` 常绿） · `UPSTREAM-USAGE.md` · 无坏引用
- [ ] Q079 · 「刻意没抄上游」的地方写清是哪几处、为什么 · `docs/` · 有清单
- [x] Q080 · `vendor/` 与 `.octop-ref/` 删掉后 `cargo build` 必须成功 —— 已做成门禁 `.vendor-freedom-check.mjs`（扫全部 Cargo.toml，禁 goose/octop 依赖键、禁指向参考源的 path/git；带 `--self-test`）· CI 已加
- [ ] Q101 · 处理 `crates/quill-testkit/fixtures/boundary/`（13 个夹具 + manifest.json）：**没有任何代码读它**（改成孤儿数据），且其中 `bnd00-all-clean`/`bnd03-floating-dep` 把 `goose = { git = … }` 当成合法/待修的样子，**与最高指示第 4 条冲突**。要么删掉，要么接回真读者 · `crates/quill-testkit/` · 二选一并说明

## I. 测试 / 健壮性 / 可诊断性

- [ ] Q081 · 每新增能力补「能证明会失败」的反向验证测试（持续项） · 全仓 · 抽查可见
- [ ] Q082 · 给关键并发路径（派工、SSE）做确定性/压力测试 · `crates/quill-server/tests/` · 有可复现测试
- [ ] Q083 · 限流（`ratelimit.rs`）的边界测试 · `crates/quill-server/` · 有测试
- [ ] Q084 · `pathsafe` 的跨平台（Windows `\\?\`）测试补齐 · `pathsafe.rs` · 有测试
- [ ] Q085 · 错误响应统一带上「下一步」指引（抽查缺的补上） · `error.rs` + `api_*` · 清单
- [ ] Q086 · 应用日志里不泄漏凭据（MCP env/headers、token） · 全仓 · 有测试
- [ ] Q087 · 健康检查 `/healthz` 覆盖新增依赖 · `routes.rs` · 字段有真来源
- [x] Q088 · 给 `scripts/status.mjs` 增加「队列进度」输出（读 `project/queue.md`，报「已完成 X / 总数 Y，下一个未完成 Qxxx」）· `scripts/status.mjs` · 已生效，并加 5 条自测（含反向：非队列行不算、带后缀编号 Q006b 不被截断）
- [ ] Q089 · 用属性测试（proptest）覆盖解析类代码（id/slug/wire） · `crates/*/tests` · 引入并跑通
- [ ] Q090 · 用 `cargo-nextest` 或等价提速全量测试 · CI · 有数据

## J. 文档与事实

- [ ] Q091 · `docs/CODE-TRUTH.md` 随代码变化更新（每条带复现命令） · `docs/` · 抽查一致
- [ ] Q092 · `docs/OCTOP-MIGRATION-INVENTORY.md` 的完成度随实现更新 · `docs/` · 抽查一致
- [ ] Q093 · `docs/ARCHITECTURE.md` 的 §1 现状随重构更新 · `docs/` · 抽查一致
- [ ] Q094 · 每个里程碑达成后，把新判据加进 `project/items.mjs` · `project/` · `status.mjs` 能判
- [ ] Q095 · `TESTSETS/` 与真机验收记录对齐 · `TESTSETS/` · 抽查一致
- [ ] Q096 · 把「已知的环境性失败」集中登记（如 Windows 无 `cat`） · `README.md`/`docs/` · 有清单
- [x] Q097 · 给本队列加「取用规则」并让自动化真按它取 —— 规则已改为「批次由用户指定（默认 1~3）」，自动化 prompt 已引用 `project/queue.md` · 本文件

## K. 长期项（不急，但别忘）

- [ ] Q098 · 桌面版（Windows/macOS/Linux）的壳与打包（需求里要） · `ui/` + CI · 能出安装包
- [ ] Q099 · 与远程 octop 协作（需求 3，现在零实现） · 设计 + 实现 · 有可演示的最小闭环
- [ ] Q100 · 本地 provider（llama.cpp / 其他）的可选接入，便于无网开发与测试 · `docs/` + 脚本 · 能起一个本地模型供测试

---

## 取用规则

1. 从**最上面未完成**的取。**批次由用户指定**（默认 1~3 条；用户说「取 30 条」就按 30 条列批，
   能一次做完的做完，做不完的留 `[ ]` 并写明卡在哪）。做完就提交（最高指示第 5 条）。
2. 判据写不出来或依赖缺失的，**不要跳过乱做** —— 在条目后补一句 `（卡在：…）` 并往下取。
3. 完成一条把 `[ ]` 改 `[x]`，并在提交信息里带上 `Qxxx`。
4. 新发现的活儿，按 `Qxxx` 续号追加，不要插队改号。
5. 本队列是**执行的队列**；里程碑级判据仍在 `project/items.mjs`，两者不要混。
6. **一条提交一个主题**：多条可以合成一个提交，但语义必须同族（如同属「删掉假入口」）。
