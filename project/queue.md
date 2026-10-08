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
- [x] Q006b · `api_chat` 的**消息读取** SQL 收口到 `chat_repo`（13 → 10 条）：`list_messages` / `last_assistant_input_tokens` / `dialog_content_chars` · `chat_repo.rs` · 已生效
- [x] Q006c · `api_chat` 剩余 **10 条**内联 SQL（`metrics` / `usage` 聚合、`prepare_turn` 发送路径的读与写）继续收口 · `crates/quill-server/src/` · **已完成 2026-10-08**：该文件 `sqlx::query` 计数 10 → 0（`grep -c 'sqlx::query' crates/quill-server/src/api_chat.rs`）；metrics/usage 与发送路径读写在 `chat_repo`，专家人格读在 `experts_repo`，`ensure_session` 改用早已存在的 `chat_repo::session_exists`；`chat_repo` 补 8 条钉 SQL 的测试（提交 ed4e60e）
- [x] Q007 · 按 `BACKLOG.md` B0-4 定 `need_str`/`opt_str` 的规范语义并合并 5+3 份 · `crates/quill-server/src/jsonx.rs` · **已完成 2026-10-08**：规范语义 = trim + 拒空串 + 带类型名 + 带位置（`where_`）；6 个文件里的 5 份 `need_str` + 3 份 `opt_str` + 1 份 `req_str` 全删，jsonx 只剩一份；41 个调用点补 `where_`；补 7 条钉语义测试（提交 3507fec）
- [ ] Q008 · `quill-server` 拆分：把 `api_*` 之外的通用件（`error.rs`/`db.rs`/`state.rs`）之外的巨石按域拆 crate 或子模块 · `crates/quill-server/src/` · 单文件上限显著下降且测试全绿 **（部分完成 2026-10-08：skillhub 族已拆 —— `skillhub.rs`(1726)+`skillhub_unpack.rs`(903) → `skillhub/` 目录模块，单文件最大 627；`fn` 名多重集 111 条前后一致、`#[test]` 59+3 不变、`quill-server` 642/0 全绿。**未打勾的原因**：>700 的非 `api_*` 文件只剩 `tools.rs`(1156) 与 `llm_providers.rs`(957)，而它们正是 Q013/Q015 要搬进 `quill-core` 的两块 —— 端口设计落地后随搬运一并解决，现在再拆属重复劳动）**
- [x] Q009 · 给 `quill-adapters` / `quill-domain` / `quill-store` 加「不许依赖上层」的编译期守卫（如文档 + CI 检查）· CI · **已完成 2026-10-08**：`.layer-guard.mjs`（依赖只能向下 + 基线清单：新违例报红、陈旧条目也报红；自测 10 条；反向验证实测报红）+ 进 gates-core（提交 9ebfeae）
- [x] Q010 · 用 `cargo-semver-checks` 或等价手段盯公开 API 兼容 · CI · **已完成 2026-10-08（选了等价手段）**：cargo-semver-checks v0.51.0 本机装过并试过，默认模式因全仓 `publish = false` 不可用（`quill-adapters not found in registry`，详见 `.api-compat-check.mjs` 头注）；改为提交在案的文本基线 `docs/api-baseline/quill-adapters.txt` + 检查脚本（自测 11 条，反向验证实测报红）（提交 9ebfeae）

## B. 内核 `quill-core`（依最高指示第 3 条：抄 goose）

- [x] Q011 · 新建 `crates/quill-core`（空壳 + 一行职责说明）· `crates/quill-core/` · **已完成 2026-10-08**：`cargo build -p quill-core` 通过（提交 6e84be1）
- [ ] Q012 · 把对话循环从 `api_chat.rs` 搬进 `quill-core`（**原样搬运**，行为不变）· `quill-core` · 测试数量不减、全绿 **（卡在 2026-10-08：对话循环直接调用 `state::AppState` / `db::DbBridge` 与 `chat_repo`/`experts_repo`/`llm`/`tools`，而 `quill-core` 不能依赖 `quill-server` —— 搬之前必须先给内核定「壳侧端口」trait（候选四类：会话历史读写 / provider 取用 / 工具注册表 / 用量记录）并由 server 实现。这是设计任务，不是机械搬运；本轮只搬了唯一不依赖壳的 mcp_client。下一步：先写端口设计 + 一个端口化的最小搬块）**
- [ ] Q013 · 把 `tools.rs` 搬进 `quill-core` · 同上 · 同上 **（卡在 2026-10-08：同 Q012 的根因 —— `tools.rs` 直接持有 `Arc<AppState>`，并调用 `api_experts::list_for_tools` / `skills_repo` / `mcp_repo` / `api_extensions`（技能正文与目录）。Q014 已示范「内核自己拥有数据模型 + 壳 re-export」（`McpServerRow`），tools 需要的是同一招的**端口版**）**
- [x] Q014 · 把 `mcp_client.rs` 搬进 `quill-core` · 同上 · **已完成 2026-10-08**：git mv 原样搬运、行为未改，测试随文件走（`quill-core` 12 条全绿；`quill-server` 相应 −13）；`McpServerRow` 一并搬进内核、`mcp_repo` 改为 re-export（提交 6e84be1）
- [ ] Q015 · 把 provider 组装（`llm_providers.rs`）搬进 `quill-core`，与 `quill-provider` 合并成一套 · 同上 · provider 组装只剩一套 **（卡在 2026-10-08：`llm_providers.rs` 里混着「存储口径」（6 条内联 SQL + `db`/`state` 依赖）与「provider 组装」；要合并成一套必须先定组装层接口（`quill-provider` 已是叶子 crate），存储部分留成 repo。属设计任务）**
- [x] Q016 · 每搬一块，在文件头标注「移植自 `vendor/goose/...` 的哪个文件」· `quill-core` · **已生效 2026-10-08（对已搬的两块）**：`mcp_client.rs` 与 `mcp.rs` 头部均写明 `vendor/goose/crates/goose/src/agents/mcp_client.rs`（含行数与版本）；`lib.rs` 把「每块都要标出处」写成纪律。后续每搬一块继续执行
- [x] Q017 · 照 `vendor/goose/crates/goose-context-management` 实现**上下文压缩**（现在只有配置字段）· `quill-core` · **已完成 2026-10-08**：`crates/quill-core/src/compaction.rs`（移植对照表逐条带 `file:line`；18 条测试含「未超阈值不许压 / 超阈值真压 / 用量分账连续 / 阈值恰好相等」；改回坏样子实测 11 条变红）。**未接线**：阈值全由参数传入，`compaction_threshold_tokens` 本模块一次没读 —— 属 Q018（提交 629529c）
- [ ] Q018 · 压缩阈值接进对话路径，并在界面上**如实反映是否生效** · `quill-core` + `ui/web` · 不再是「只存不读」
- [ ] Q019 · 照 goose 实现**记忆**（当前为零）· `quill-core` · 有可验证的读写回路 **（卡在 2026-10-08：**参考源本身不确定** —— goose v1.53.0 里没有独立的 memory crate 或模块（`ls vendor/goose/crates/` 无 memory；`grep -rl memory vendor/goose/crates/goose/src/` 命中的是 `acp/`、`platform_extensions/chatrecall.rs` 等，都不是「记忆」原语）。要先定「quill 的『记忆』对齐 goose 的哪一块」（候选：`session/` 的会话记忆、`chatrecall` 平台扩展、`context_mgmt` 的长期摘要），再动手 —— 否则就是自创，违反第 3 条。另外读写回路需要存储端口，同 Q012）**
- [x] Q020 · 照 `vendor/goose/crates/goose/src/agents/` 补**状态机** · `quill-core` · **已完成 2026-10-08**：`crates/quill-core/src/state_machine.rs`（9 状态 × 10 事件，转移带 goose `file:line`；非法转移返回 `TransitionError` 不静默通过；文档列出未搬的 17 个 ops 及原因；8 条测试，改坏预算判定实测 2 条变红）。**未接线**：还没接进 `api_chat` 的真实路径（属 Q012/Q018）（提交 52e17ea）
- [ ] Q021 · 照 goose 补**快照**（snapshots） · `quill-core` · 能回滚到某一轮 **（卡在 2026-10-08：**参考源不存在** —— `ls vendor/goose/crates/goose/src/agents/snapshots/` 里只有 3 个 insta 测试快照文件（`.snap`），不是功能；`grep -rn "snapshot" vendor/goose/crates/goose/src/ --include=*.rs` 的命中全是「配置快照 / provider 清单快照 / 测试快照」，**没有会话回滚**这类原语。要补「能回滚到某一轮」只有两条路：① 按 quill 自己的 schema 已有列做（`0001_init.sql:264-265` 的 `checkpoint_key`/`checkpoint_path` + `:406-407` 的唯一索引，Rust 侧零读写）—— 那是**自创**，违反第 3 条；② 先定「对齐 goose 的哪一块」（候选：`session/` 的可重放 message 流 + 外部 git 工作区快照），属设计任务。两条都需先拍板，本轮不做）**
- [x] Q022 · 照 goose 补**重试**（retry.rs） · `quill-core` · **已完成 2026-10-08**：`crates/quill-core/src/retry.rs`（1007 行 + 19 条测试）。**参考源已更正**：真正的上游重试不在 `agents/retry.rs`（那是 agent 层整轮重试，无 5xx/429 分类与退避公式），而在 `goose-provider-types/src/retry.rs`（默认 3 次 / 1s / ×2 / 30s 上限 + 抖动 0.8~1.2）与 `goose-providers/src/http_status.rs`（status→错误映射、429 `Retry-After`），模块头逐条 `file:line`。反向验证：把 4xx 判成可重试 → 1 条红；去掉退避上限 → 3 条红。**未接线**（接 provider 路径属 Q012）
- [ ] Q023 · 子 agent：`member_executor` 的 `steer`（运行中追加指令） · `quill-core` · 不再返回「尚未实现」 **（卡在 2026-10-08：**当前执行模型里没有「运行中」这个窗口** —— `ProviderMemberExecutor::start` 在**返回 future 之前就把活干完**（`member_executor.rs:178-181` 的注释与实现），因为派工侧用空 waker 自旋 poll（`quill-agent/src/dispatch.rs`）。成员在调用方拿到 future 时已经跑完，`steer`/`abort` 没有可作用的对象。要先改**派工执行模型**（真异步 + 运行中成员注册表 + 取消令牌），那是 Q012 端口设计的同一片地）**
- [ ] Q024 · 子 agent：`member_executor` 的 `abort`（中途取消） · `quill-core` · 同上 **（卡在 2026-10-08：同 Q023 的根因 —— 没有「运行中」窗口可取消；`member_executor.rs:217` 的「abort 尚未实现」是诚实报错，改成 `Ok(())` 才是撒谎）**
- [ ] Q025 · 子 agent：给成员各开**独立会话**（对齐 goose 的「每子 agent 独立 config + session」） · `quill-core` · 成员产出进各自的会话 **（卡在 2026-10-08：依赖两件未做的事 —— ① 派工执行模型改造（同 Q023）；② 成员会话的落库口径（`MemberOutcome` 只有 `member/status/completed_scope/output`，没有会话归属；要写进 `sessions`/`messages` 就得先定「成员会话算不算用户可见会话」，这直接影响侧栏与用量页 —— 见 Q052/Q026））**
- [ ] Q026 · 成员执行也记 **token 用量**（现在 `MemberOutcome` 不带 usage） · `quill-core` · 用量页能看到派工消耗 **（卡在 2026-10-08：`ChatResponse.usage` 拿得到（`quill-provider`），但**记到哪要产品决策** —— 候选 A：写进成员会话的 `messages`（用量页天然能看到，但会出现在该会话的消息列表与喂给模型的历史里，用户可见行为变化）；候选 B：新增派工用量表 + 迁移 + 用量页聚合 + 前端展示（改动面大，且要动 `ui/web`）。两条都不是纯技术选择，先定口径）**
- [x] Q027 · 照 goose 的 `permission` 对齐权限模型（现在 quill-control 是多租户口径，需比对） · `quill-core` · **已完成比对 2026-10-08**：结论 = goose 是「单用户 agent 对工具调用的许可」（模式级/工具级/单次调用级等 9 类，`permission/` 4 文件 820 行），quill **一个对应物都没有**（`grep -rniE "approval|permission" crates/` 50 行里除 quill-backup 的 `PermissionDenied` 与 `quill-core` 状态机文档外，只剩 3 条写死的 `approval_mode:"manual"` 字符串、1 条契约断言、1 个零读写的列）；全仓唯一工具执行点 `api_chat.rs:1049 registry.call(call)` **无任何门禁**。逐条对照与「值不值得补」见 `docs/KERNEL-ALIGNMENT.md §Q027`
- [x] Q028 · 照 goose 的 `slash_commands` 实现斜杠命令（先核 quill 有没有） · `quill-core` · **已完成比对 2026-10-08（结论：quill 没有实现，0 行）**：`crates/` + `ui/web/src/` 的 15 处命中全是注释 / 测试数据 / 路径校验，服务端链路（`chatApi.ts:40 → routes.rs:116 → api_chat.rs post_message → run_turn`）无任何命令分支；goose 侧是跨 12 文件 / 19 处引用的接线链。逐条对照表与「缺口 → 落点」见 `docs/KERNEL-ALIGNMENT.md §Q028`。**实现尚未做**（若要做，按取用规则第 4 条新开条目）
- [x] Q029 · provider 移植覆盖度逐块比对（`quill-provider` vs `goose-providers`） · `docs/` · **已完成 2026-10-08**：`docs/KERNEL-ALIGNMENT.md §Q029` 出覆盖表（30 行：已覆盖 2 / 已覆盖但两套 1 / 部分 15 / 未覆盖 12）+ 差集表；并核实 §1.6 的「两套」成立（`quill-provider` 3241 行传输层 vs `llm_providers.rs`+`llm.rs` 1345 行 DB 配置 / 组装，真正重复只有 3 处：`/models` 探测、base_url 归一、传输错误分类）
- [x] Q030 · 会话模型对齐 goose 的 `session`（当前 `api_chat` 自研） · `quill-core` · **已完成比对 2026-10-08**：12 条维度里**等价 0 / 部分 5 / 缺失或语义相反 7** —— goose 的会话是「可重放的完整 message 流 + 元数据」（工具往返也是消息、可见性在 message metadata、状态由消息尾部推导），quill 是「一张表 + 一问一答两条消息」（工具往返不落库、`sessions.state` 插入时写死 `'IDLE'` 后再没动过、`error_state`/`replan_count`/`summary*`/`compacted_count`/`checkpoint_*` 列在 Rust 侧零读写）。§4 点名 `docs/ARCHITECTURE.md:106` 的「api_chat 自研」具体差在哪，并给出第一步落点（让工具往返入 `messages`）· `docs/KERNEL-ALIGNMENT.md §Q030`

## C. 把 501 桩接成真接口（`routes.rs` 现有 11 处）

- [x] Q031 · `POST /api/experts/import` 真实现 · `api_experts.rs` · **已完成 2026-10-08**：导出/导入各 7 字段（`id/display_name/description/instructions/model/source_template/default_enabled`）+ `bundle_version:1`；逐条结果（created/updated/skipped/failed + 原因 + summary + `ok`），**不是全量替换**；写入复用 `experts_repo::sql_put` 的 upsert（未另写 SQL）；内部列（`owner_user_id`/`asset_hash`/`persona_hash`/`visibility`/`is_builtin`/时间戳）在 `redacted` 里逐条点名不带出；内置系统专家不进清单也碰不到。往返测试：跨账号与同账号两条路都**逐字段等价**（无需排除任何字段）；反向验证（export 少写 `source_template` / import 少读 `source_template`）实测变红
- [x] Q032 · `GET /api/experts/export` 真实现 · `api_experts.rs` · **已完成 2026-10-08**：与 import 对称（导出的输出可**原样喂回**导入；`redacted`/`note` 被接受而不是当未知字段拒掉）。501 桩已换成真 handler（`routes.rs` 前后对比见提交）
- [ ] Q033 · `POST /api/wiki/ingest` 真实现（依赖 Q0xx 知识库） · `api_wiki.rs` · 真写入索引
- [ ] Q034 · `POST /api/wiki/query` 真实现 · `api_wiki.rs` · 真返回答案
- [x] Q035 · `POST /api/wiki/search` 真实现（只读索引，可不依赖 LLM） · `api_wiki.rs` · **已完成 2026-10-08**：真读 `index.md`（`WikiStore::read_index`），复用现成的 `WikiIndex::lookup` 计分（标题 +2 / 摘要 +1，同分按标题升序，有测试钉住不漂）；返回带**命中依据**（`matched_fields` + 每词命中处前后 24 字符摘录）、`total_hits` 与 `truncated`（真数、不隐藏）；**不返回 `path` 与 `page_type`** —— 索引里没有路径列、解析出的类型恒为 `Summary`，给出来就是假数据（注释与测试都写明）；索引不存在时 `index_present:false` 且说明「零命中 ≠ 资料库没有」。8 条单测（含「删光页面文件仍命中」证明只读索引）+ 反向验证（去掉摘要命中 → 2 条红）。路由已接：`post(crate::api_wiki::search)`
- [x] Q036 · `PATCH /api/extensions/mcp/{name}` 真实现 · `api_extensions.rs` · **已完成 2026-10-08**：改名走 `UPDATE`（保留 `created_at` 与行身份，有测试钉住），不是 delete+insert；**缺省字段 = 不变、显式 `null` = 清空**（`enabled_capabilities: null` = 全禁，是取值不是「不变」）；与 POST 共用同一份传输交叉校验；用户隔离（别人的行 404，同名按用户各一份）；撞名（含软删行占名）409 且不静默合并；`updated_at` 只在名字或指纹真变时才写。`mcp_repo` 加 `get`/`apply_patch`（`PatchOutcome` 四态）+ 8 条测试，`tests/extensions_http.rs` 9 条真库往返；反向验证（改名当没改 / `truncated` 恒 false / 去掉 `user_id` 谓词）实测变红。路由已接：`patch(api_extensions::patch_mcp)`
- [x] Q037 · `GET /api/extensions/plugins` —— **刻意保持 501**：前端 `capabilityGaps.ts` 已把它登记为「已登记路由、处理函数未实现」并如实显示给用户；删掉会让那句「已登记」变假话（退化成 404）。已在路由处加注释说明 · `routes.rs`
- [x] Q038 · `GET /api/upgrade/check` 真实现 · `api_upgrade` · **已完成 2026-10-08**（admin-only）：当前版本取本 crate 的 `env!("CARGO_PKG_VERSION")`（与 `GET /api/version` 同源）；「是否有新版」只认**真来源** `QUILL_UPGRADE_MANIFEST_URL`（`{version,url?,notes?}` JSON，10s 超时 / 重定向≤5 / 响应体 256 KiB 封顶）。**没配来源、取不到、JSON 坏 → 一律 `has_update: null` + note 说明「这是不知道，不是已是最新」**；`false` 只在真取到清单且真比出不高于当前版本时出现。版本比较规则（允许一个前导 v、按段纯十进制、缺段补 0、非数字/溢出→不可比较→null）原样写进响应 `VERSION_RULE`
- [x] Q039 · `POST /api/upgrade/prepare` 真实现（含升级前备份守卫） · 同上 · **已完成 2026-10-08**（admin-only，201）：真调用 `quill_upgrade::take_pre_upgrade_backup`（Q064 记的「零调用方」就此结束），备份落在与备份路由**同一个** `实例根/backups`，所以这份升级前备份能被 `/api/backup/verify` 校验、被 `quill restore` 回滚。**修掉一个真 bug**：原来的「先 exists 再 join」有 TOCTOU，同毫秒两次 prepare 会共用一个目录名（实测第二个 `VACUUM INTO` 报 `table schema_version already exists`，备份内容不可信）；现在用 `create_dir` 原子预留名字。响应含备份目录/摘要（`db_sha256`/文件数/字节数）/真实回滚命令/`upgraded:false`（明说没升级）；失败 503/409/500 + 既有中文阻断文案，**绝不返回成功**
- [x] Q040 · `GET /api/upgrade/history` 真实现 · 同上 · **已完成 2026-10-08**（admin-only）：读 `<数据根>/upgrade-history.jsonl`（每次 prepare 成功追加一行：时间/版本/备份目录/摘要）。**文件不存在 → 200 + 空列表 + note「还没有升级记录……不是读失败」**；文件在但读不出或某行坏 → **500 并点名文件与行号**（静默跳过等于造一份少一条记录的假历史）
- [x] Q041 · `GET /api/ws` 真实现或如实删掉 —— **选了删**：核过前端 `ui/web/src` 与全部测试都**没人用**它，它是一条假接口（最高指示第 2/5 条：不画没有后端的入口）· `routes.rs`

## D. octop 外壳迁移（依最高指示第 2 条）

- [ ] Q042 · `/api/cron` 有真路由（自动化页现在配的是假后端） · `crates/quill-server/src/` · 页面上的开关真有用
- [ ] Q043 · 团队限制列 `max_dispatch`/`max_replan`/`max_ask_depth`/`guidelines` 不再只存不读 · `quill-agent`/`api_teams.rs` · 生产侧真有引用
- [ ] Q044 · 不再上报没有真来源的字段（`plugins`/`wiki_index`/`skill_count`/`tool_allowlist`） · `api_admin.rs` 等 · 字段有真来源或删掉
- [ ] Q045 · 通道（channels）按 TencentCloud/Octop 重做 · `api_channels.rs` · 有 `file:line` 对齐
- [ ] Q046 · 个性化页面按 octop 重做 · `ui/web/src/personalization` · 同上
- [ ] Q047 · 专家市场（列表 + 安装）接通 · `api_expert_market.rs` · 真能装
- [x] Q048 · 备份三条路由只允许 admin 的**回归测试**钉住 · `crates/quill-server/tests/` · **已完成 2026-10-08**：核到三条路由（export/restore/verify）都走 `RequireAdmin`，非 admin + 有效令牌 → **403**（`forbidden`，无 401/403 差异）。新增 4 条：三条逐路由 403（各先造一份**真存在的备份**，确保 403 来自鉴权而不是「目录不存在」）+ 一条正向 admin 最小往返（export 201 真落盘 → verify 200 `ok:true` → restore 200）。**反向验证**：把 `auth.rs` 的 `if !ctx.is_admin` 临时改成 `if false` → 4 条全红（且红的方式是「回出了 db_sha256 与绝对路径」），还原后绿
- [ ] Q049 · `POST /api/backup/restore` 在演练里真还原成功 · `api_backup.rs` · 端到端可验
- [ ] Q050 · 客户端不能指定落盘位置（`../` 与盘符一律拒）的测试 · `api_backup.rs` · 已有则补反向验证
- [x] Q051 · 会话改名：`PATCH /api/sessions/{id}` 注册（现在 405） · `routes.rs`/`api_chat.rs` · **已完成 2026-10-08**：新增 `api_chat::rename` + `chat_repo::rename_session`（`RENAME_SQL` 带 user_id/deleted_at 谓词、**刻意不碰 `last_active_at`** —— 改名不是「用过它」，刷新活跃时间会让会话在侧栏凭空跳到最上面）；标题上限抽成 `TITLE_MAX_CHARS`（建会话与改名**共用同一个 64**，此前 create 里是内联的 64）；空白标题 400（走 Q007 的规范取值助手）、超长按**字符**截断、未知字段 400、软删/别人的 → 404。6 条 HTTP 测试 + 1 条 SQL 测试；**反向验证**：把 rename 换成不落库的 no-op → 「改名必须真落库」实测变红，还原后 6/6 绿
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
