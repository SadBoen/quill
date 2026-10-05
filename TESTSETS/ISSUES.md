# 问题记录

每条按这个格式记。**只修不留痕等于没修** —— 下次谁都不知道这里踩过什么。

判定边界见 `README.md`：**答不上来不算 bug**（本机 4B 模型），
链路断、状态说谎、错误被吞、隔离失效才算。

---

## 格式

```
## ISSUE-nnn · 一句话标题

- **发现于**：任务 id / 手工浏览 / 门禁
- **现象**：用户看到什么。写用户视角，不写代码视角。
- **严重度**：阻断 / 严重 / 一般 / 轻微
- **根因**：为什么会这样。指到文件与行。
- **复现**：一步步怎么触发。
- **修复**：改了什么，为什么这么改（有没有更小的改法，为什么不用）。
- **回归**：钉住它的测试叫什么；全量门禁多少 passed / 0 failed。
- **状态**：待修 / 已修待回归 / 已修已回归 / 挂起（附挂起理由）
```

---

## ISSUE-000 · 迁移在多连接池上必炸（已修）

- **发现于**：接入 MCP 配置存储后跑全量门禁
- **现象**：`quill doctor` 退出码 2，报
  `there is already another table or index with this name: mcp_servers`；
  6 个 CLI 用例全红。**注意这是真实服务才有的现象**，测试里一直绿。
- **严重度**：阻断
- **根因**：`quill_store::run_migration` 逐条 `sqlx::query(s).execute(pool)`，
  而 `SqlitePool` 每条 query 各自取连接。0007 开头的
  `PRAGMA foreign_keys = OFF` 落在连接 A，紧接着的 `DROP TABLE` / `ALTER TABLE`
  却在连接 B 上照旧开着外键。
  **为什么测试没抓到**：`quill-store` 的迁移测试用 `in_memory()`，
  那条池 `max_connections(1)`，单连接下 PRAGMA 永远生效；真实服务是
  `configure_pool(path, 5)`，五连接，问题必现。
- **复现**：`configure_pool(path, 5)` 建池，握住一条连接不放，再
  `migrate(&pool)`。
- **修复**：`run_migration` 固定用一条连接并包在一个事务里。顺带失败时
  错误带「第 N/M 条语句 + 语句片段」。
  **为什么不逐条提交**：一条迁移要不就全成要不就全不成；逐条提交会留下
  半张表，而台账还没写，下次启动当没跑过再来一遍。
- **回归**：`all_migrations_apply_through_a_multi_connection_file_pool`、
  `a_failing_migration_leaves_nothing_behind`、
  `a_failing_statement_says_which_one_it_was`
- **状态**：已修已回归

## ISSUE-001 · 指纹长度会让每一行写入被数据库拒掉（已修）

- **发现于**：写 `mcp_repo` 时自查 schema
- **现象**：每一行 INSERT 都被拒，而错误只说
  `CHECK constraint failed`，不告诉你是哪一列。
- **严重度**：阻断
- **根因**：`mcp_servers.asset_hash` 与 `skills.content_hash` 的 CHECK
  都是 `length = 32`，我截 SHA-256 到 16 字节。**编译过、单测也过**，
  只有真写库才炸。
- **修复**：复用 `db::digest32`（本来就返回 32 字节，`experts` 在用）。
  顺带把分段改成「个数 + 每项长度前缀」——`[""]` 与 `[]`、
  `["a\u{1}b"]` 与 `["a","b"]` 这两组是被自己的单测抓出来的真碰撞。
  另外 `env` 与 `headers` 拆成两个区段算，合成一张 map 会让同名字段互相覆盖。
- **回归**：`the_asset_hash_is_the_length_the_check_constraint_demands`、
  `field_boundaries_cannot_be_forged_by_shifting_a_delimiter`、
  `env_and_headers_are_hashed_as_two_separate_sections`、
  `the_capability_three_states_are_three_different_fingerprints`
- **状态**：已修已回归

## ISSUE-002 · 「保存 → 重新编辑」这一圈是死的（已修）

- **发现于**：浏览器实测设备页
- **现象**：保存后想编辑这条配置，页面直接拦下提交，浏览器给一个
  没有来由的红框。
- **严重度**：严重
- **根因**：前端 `<input name="name">` 的 `pattern` 是
  `[a-z][a-z0-9_]{0,31}`（只允许下划线），服务端把 `_` 归一成 `-` 并回填
  `company-search`。回填的值过不了页面自己的校验。旧写法还要求首字符是
  字母，而服务端允许数字开头。
- **复现**：设备页新增一条名为 `Company Search` 的服务 → 保存 → 点编辑 →
  改任何东西 → 点保存 → 被拦。
- **修复**：`MCP_NAME_PATTERN` 改成 `[a-z0-9]([a-z0-9-]{0,62}[a-z0-9])?`。
  在 `mcp_repo.rs` 里加了同源表子，方向是**「服务端产出 ⊆ 前端能收」**——
  反过来不成立：前端比服务端严一点是提前拦下，严过头才是 bug。
- **回归**：`every_normalized_name_passes_the_frontend_pattern`（Rust）、
  `mcpConfig.test.ts` 的 4 条
- **状态**：已修已回归

## ISSUE-003 · 界面把已经接通的路由说成 501（已修）

- **发现于**：接完 MCP/SKILL 后回头查文案
- **现象**：对话页工具坞与专家库生成面板都把
  `GET /api/extensions/{mcp,skills}` 标成「未接通 · 501」，而这两条路由
  实际返回 200。
- **严重度**：严重（对着用户说谎）
- **根因**：状态是硬编码在两张前端表里的，后端接通了没人回来改。
- **修复**：抽到 `ui/web/src/capabilityGaps.ts` 单一数据源，两处共用；
  文案分 `partial`（存储层已通、执行层没接）与 `not-implemented`
  （路由压根不存在）两类。
- **回归**：`capabilityGaps.test.ts`、`LibraryTab.test.tsx` 新增那条
- **状态**：已修已回归

## ISSUE-004 · 删不存在的 MCP 服务仍报「已删除」（已修）

- **现象**：`DELETE /api/extensions/mcp/{name}` 对不存在的名字也返回
  `"deleted": true`。
- **严重度**：一般
- **根因**：实现走「读全量 → 去掉目标 → 写回」，无论目标在不在都写 `true`。
- **修复**：按过滤前的列表判断目标是否真的存在。
- **回归**：`delete_reports_honestly_when_there_was_nothing_to_delete`
- **状态**：已修已回归

## ISSUE-005 · 设备页文案说「服务端会回 501」（已修）

- **现象**：设备页顶部写着「quill 尚未实现该路由的实现体时，服务端会直接回
  501」。路由早就实现了。
- **严重度**：一般
- **根因**：同一批硬编码文案。
- **修复**：改成读服务端返回的 `connected` 与 `note`，原样显示；
  组件里**不许**出现任何由 `servers.length` 推断出来的「已连接」字样。
- **回归**：`saving_mcp_never_claims_to_be_connected`（HTTP 层）、
  `McpLinkStatus` 组件
- **状态**：已修已回归

---

## 待补（还没跑到，先占位）

跑 100 条任务时新发现的问题往这里追加，编号接着往下排。
**不要**把新问题混进上面已修的条目里 —— 它们的回归测试不同。
