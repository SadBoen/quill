-- ════════════════════════════════════════════════════════════════════════════
-- Quill v1 schema
--
-- 实测环境：SQLite 3.46.1（44 条语句全部干净应用；41 项隔离性断言全绿）
--   ⚠️ 2026-10-05 接手复核：原写「3.53.1」有误，本机实测为 3.46.1
--      （Python 3.13.5 内置 sqlite3 模块；WSL 无 sqlite3 CLI，故全部走模块）。
--      44 条 = 14 CREATE TABLE + 22 CREATE INDEX + 8 CREATE UNIQUE INDEX，已复核。
-- 验证脚本（已入库，不再是 .scratch 里的悬空声明）：
--   crates/quill-store/verify/run.sh            一键跑全部，rc≠0 即失败
--   crates/quill-store/verify/verify.py          41 项隔离性/约束/执行计划断言
--   crates/quill-store/verify/assert_invites.py  17 项 invites 结构与「零写路径」断言
--   crates/quill-store/verify/retrieval_final.py 15 查询词召回率对照实验（非断言，rc 恒 0）
--   三者均以 __file__ 推算路径，直接指向本文件，无副本、不依赖任何外部 CLI。
--
-- 约定：
--   BLOB id      = uuid v7 原始 16 字节（时间有序 → B-tree 追加写入不分裂）
--   INTEGER 时间  = Unix epoch 毫秒 UTC（SQLite 无 DATE 类型；INTEGER 比较最快）
--   全部 STRICT  = 写入时类型错配立即报错，不留到查询期
--   全部软删除    = deleted_at（NULL = 存活），唯一索引用部分索引
--
-- ┌─────────────────────────────────────────────────────────────────────────┐
-- │ 检索层说明（曾被误判为「缺 FTS5 虚表 = 检索功能不存在」，此处留档）      │
-- │                                                                         │
-- │ 本 schema 【不】建 FTS5 虚表，这是实测结论而非遗漏：                    │
-- │   方案                              召回率    约束能力                   │
-- │   ────────────────────────────────  ────────  ──────────────────────    │
-- │   FTS5 default(unicode61)             0/15    —                          │
-- │   FTS5 trigram                        4/15    —                          │
-- │   FTS5 + jieba 预分词                11/15    STRICT/CHECK/FK 全不支持 │
-- │   ★ wiki_index 自建倒排 + BM25       11/15    全部支持                  │
-- │   （实测脚本 verify/retrieval_final.py，15 个查询词全部出自原文）       │
-- │                                                                         │
-- │ 三条否决理由：                                                          │
-- │   1. FTS5 是【精确 token 匹配】，不做子串召回。索引存「知识库设计」      │
-- │      时查「知识」返回 0（实测）。⚠️ 但「知识」在 wiki_index 上同样      │
-- │      返回 0（实测 4/15 未命中含「知识」）：根因是分词边界 —— 3 字复合词│
-- │      吸收了 2 字查询词，wiki_index 本身不能凭空补回没入库的词。          │
-- │      正确表述是「wiki_index【可以】做子串召回，但前提是入库端同时索引   │
-- │      2-gram 与复合词」（见下方 wiki_index 段与 quill-wiki 分词器约定）。 │
-- │   2. 虚表拿不到数据库约束。实测 CREATE VIRTUAL TABLE ... CHECK(x)       │
-- │      → "parse error"。而 user_id 隔离正是靠 CHECK/FK/复合主键实现的     │
-- │      （见 messages 的复合外键）。引入虚表 = 开一个不受保护的口子。        │
-- │   3. 加了它还要保证两套索引一致 —— 重建顺序、失败处理、对账全部翻倍。    │
-- │                                                                         │
-- │ BM25 在 wiki_index 上完全可执行：SQL 取倒排，Rust 侧打分。             │
-- │ 若未来 wiki 规模到 BM25 变慢（真实瓶颈），正确做法是换 tantivy          │
-- │ （真倒排库，支持中文 analyzer），而不是补 FTS5 虚表。                    │
-- └─────────────────────────────────────────────────────────────────────────┘
--
-- ┌─────────────────────────────────────────────────────────────────────────┐
-- │ ⚠️ busy_timeout —— 为什么「为什么」要写在 DDL 文件里                       │
-- │                                                                         │
-- │ 实际 PRAGMA 设置在 Rust 侧连接池（quill-store/src/pool.rs），因为        │
-- │ PRAGMA 是【连接级】的、不是【数据库级】的 —— 它不被记在数据库文件里。     │
-- │ 但「为什么必须设」属于 schema 约束，写在这里防止后人误删：              │
-- │                                                                         │
-- │   1. 写池 size=1 时，默认 busy_timeout=0 → 第 2 个并发写者立即          │
-- │      SQLITE_BUSY 报错，而不是等待。配 5000ms = 等 5 秒。                 │
-- │   2. journal_mode=WAL 与 busy_timeout 必须【同批 PRAGMA 设置】。        │
-- │      若 pool.rs 里 WAL 已生效但 busy_timeout 因某分支漏设，             │
-- │      WAL 单写者的排队等待就退化成立即失败 —— 静默劣化，不报错。          │
-- │   3. 两者顺序也有讲究：先 journal_mode=WAL（改写数据库头，              │
-- │      需要独占锁），再 busy_timeout（纯连接属性）。                      │
-- │                                                                         │
-- │ 必测断言（见 verify.py 与 backend-engineer 的连接池测试）：              │
-- │   对连接池中【每一条连接】断言 PRAGMA foreign_keys = 1                   │
-- │   —— foreign_keys 默认是 OFF，漏开则本文所有复合外键全部形同虚设。       │
-- └─────────────────────────────────────────────────────────────────────────┘
-- ════════════════════════════════════════════════════════════════════════════

CREATE TABLE schema_version (
  version     INTEGER NOT NULL PRIMARY KEY,
  name        TEXT    NOT NULL,
  checksum    BLOB    NOT NULL,
  applied_at  INTEGER NOT NULL,
  exec_ms     INTEGER NOT NULL,
  app_version TEXT    NOT NULL,
  note        TEXT    NOT NULL DEFAULT ''
) STRICT;

-- ─────────────────────────── control 域 ───────────────────────────

CREATE TABLE users (
  id               BLOB    NOT NULL PRIMARY KEY,
  username         TEXT    NOT NULL,
  username_norm    TEXT    NOT NULL,
  display_name     TEXT    NOT NULL,
  password_hash    BLOB    NOT NULL,
  password_salt    BLOB    NOT NULL,
  -- ⚠️ 曾经这里写着 DEFAULT 'argon2id'，但实现的算法是 PBKDF2-HMAC-SHA256
  --    （见 crates/quill-control/src/password.rs 模块文档）。
  --    一个"默认写成没实现的算法"的 schema 比没有 schema 更危险：漏写该列的行
  --    会**静默**存下 argon2id，而验证端根本不认这个值——用户永远登不进去，
  --    且没有任何一行报错。故这里**刻意不给默认值**：漏写列 = 立即 INSERT 失败。
  -- 取值不枚举：合法值由 quill-control 的 PasswordHasher::algo_tag() 产出，
  -- schema 不认识 Rust 常量，枚举即漂移（铁律：验证器不得与被验证对象共享真相源）。
  password_algo    TEXT    NOT NULL,
  role             TEXT    NOT NULL,
  status           TEXT    NOT NULL DEFAULT 'active',
  locale           TEXT    NOT NULL DEFAULT 'zh-CN',
  token_epoch      INTEGER NOT NULL DEFAULT 1,
  settings_json    TEXT    NOT NULL DEFAULT '{}',
  pwd_changed_at   INTEGER NOT NULL,
  last_login_at    INTEGER,
  login_fail_count INTEGER NOT NULL DEFAULT 0,
  locked_until     INTEGER,
  created_at       INTEGER NOT NULL,
  updated_at       INTEGER NOT NULL,
  deleted_at       INTEGER,
  CHECK (length(id) = 16),
  CHECK (length(username_norm) BETWEEN 1 AND 64),
  CHECK (length(display_name) BETWEEN 1 AND 64),
  CHECK (role IN ('owner','member')),
  CHECK (status IN ('active','disabled')),
  CHECK (length(password_algo) > 0),
  CHECK (login_fail_count >= 0),
  CHECK (deleted_at IS NULL OR deleted_at >= created_at)
) STRICT;

CREATE UNIQUE INDEX ux_users_username_norm
  ON users (username_norm) WHERE deleted_at IS NULL;

CREATE TABLE sessions_auth (
  id            BLOB    NOT NULL PRIMARY KEY,
  user_id       BLOB    NOT NULL,
  token_hash    BLOB    NOT NULL,
  family_id     BLOB    NOT NULL,
  parent_id     BLOB,
  issued_at     INTEGER NOT NULL,
  expires_at    INTEGER NOT NULL,
  rotated_at    INTEGER,
  revoked_at    INTEGER,
  revoked_reason TEXT,
  user_agent    TEXT,
  peer_addr     TEXT,
  created_at    INTEGER NOT NULL,
  CHECK (length(id) = 16),
  CHECK (length(token_hash) = 32),
  CHECK (expires_at > issued_at),
  CHECK (revoked_at IS NULL OR revoked_at >= issued_at),
  FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
) STRICT;

CREATE UNIQUE INDEX ux_auth_token_hash ON sessions_auth (token_hash);
CREATE INDEX ix_auth_user_live ON sessions_auth (user_id, expires_at)
  WHERE revoked_at IS NULL;
CREATE INDEX ix_auth_expire ON sessions_auth (expires_at);
CREATE INDEX ix_auth_family ON sessions_auth (family_id);

CREATE TABLE invites (
  id          BLOB    NOT NULL PRIMARY KEY,
  code_hash   BLOB    NOT NULL,
  created_by  BLOB    NOT NULL,
  role        TEXT    NOT NULL DEFAULT 'member',
  max_uses    INTEGER NOT NULL DEFAULT 1,
  used_count  INTEGER NOT NULL DEFAULT 0,
  expires_at  INTEGER NOT NULL,
  revoked_at  INTEGER,
  accepted_by BLOB,
  accepted_at INTEGER,
  created_at  INTEGER NOT NULL,
  CHECK (length(code_hash) = 32),
  CHECK (role IN ('owner','member')),
  CHECK (used_count <= max_uses),
  -- 邀请码必须能在被接受前过期；否则建出来就是「一出生就过期」的废数据
  CHECK (expires_at > created_at),
  FOREIGN KEY (created_by) REFERENCES users(id) ON DELETE CASCADE,
  FOREIGN KEY (accepted_by) REFERENCES users(id) ON DELETE SET NULL
) STRICT;

CREATE UNIQUE INDEX ux_invites_code ON invites (code_hash);

-- ─────────────────────────── ext-hub 域 ───────────────────────────

CREATE TABLE experts (
  id             TEXT    NOT NULL,
  owner_user_id  BLOB    NOT NULL,
  display_name   TEXT    NOT NULL,
  version        TEXT    NOT NULL,
  description    TEXT    NOT NULL,
  role_summary   TEXT    NOT NULL DEFAULT '',
  visibility     TEXT    NOT NULL,
  tool_policy_json TEXT  NOT NULL,
  tags_json      TEXT    NOT NULL DEFAULT '[]',
  license        TEXT    NOT NULL,
  default_enabled INTEGER NOT NULL DEFAULT 1,
  is_builtin     INTEGER NOT NULL DEFAULT 0,
  asset_hash     BLOB    NOT NULL,
  persona_hash   BLOB    NOT NULL,
  skill_count    INTEGER NOT NULL DEFAULT 0,
  created_at     INTEGER NOT NULL,
  updated_at     INTEGER NOT NULL,
  deleted_at     INTEGER,
  PRIMARY KEY (owner_user_id, id),
  CHECK (id GLOB '[a-z0-9]*' AND id NOT GLOB '*[^a-z0-9-]*' AND id NOT GLOB '*..*'),
  CHECK (length(id) BETWEEN 1 AND 64),
  CHECK (length(display_name) BETWEEN 1 AND 64),
  CHECK (visibility IN ('default_visible','manual_enable','builtin_system','user_authored')),
  CHECK (tool_policy_json IS NOT NULL AND length(tool_policy_json) >= 2),
  CHECK (tags_json IS NOT NULL AND length(tags_json) >= 2),
  CHECK (default_enabled IN (0,1)),
  CHECK (is_builtin IN (0,1)),
  CHECK (length(asset_hash) = 32),
  CHECK (skill_count >= 0),
  CHECK ((is_builtin = 1) = (owner_user_id = x'00000000000000000000000000000000'))
) STRICT;

CREATE INDEX ix_experts_visible ON experts (visibility, default_enabled, id)
  WHERE deleted_at IS NULL;
CREATE INDEX ix_experts_owner_recent ON experts (owner_user_id, updated_at DESC)
  WHERE deleted_at IS NULL;

CREATE TABLE mcp_servers (
  user_id            BLOB    NOT NULL,
  name               TEXT    NOT NULL,
  transport          TEXT    NOT NULL,
  command            TEXT,
  args_json          TEXT    NOT NULL DEFAULT '[]',
  env_json           TEXT    NOT NULL DEFAULT '{}',
  url                TEXT,
  headers_json       TEXT    NOT NULL DEFAULT '{}',
  secret_refs_json   TEXT    NOT NULL DEFAULT '[]',
  tool_allowlist_json TEXT   NOT NULL DEFAULT '[]',
  enabled            INTEGER NOT NULL DEFAULT 1,
  timeout_ms         INTEGER NOT NULL DEFAULT 30000,
  description        TEXT    NOT NULL DEFAULT '',
  asset_hash         BLOB    NOT NULL,
  created_at         INTEGER NOT NULL,
  updated_at         INTEGER NOT NULL,
  deleted_at         INTEGER,
  PRIMARY KEY (user_id, name),
  CHECK (name GLOB '[a-z0-9]*' AND name NOT GLOB '*[^a-z0-9-]*' AND name NOT GLOB '*..*'),
  CHECK (length(name) BETWEEN 1 AND 64),
  CHECK (transport IN ('stdio','http','builtin')),
  CHECK (enabled IN (0,1)),
  CHECK (timeout_ms BETWEEN 1000 AND 600000),
  CHECK (length(asset_hash) = 32),
  CHECK (transport <> 'stdio' OR command IS NOT NULL),
  CHECK (transport <> 'http'   OR url IS NOT NULL),
  FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
) STRICT;

CREATE INDEX ix_mcp_enabled ON mcp_servers (user_id, enabled)
  WHERE deleted_at IS NULL;

CREATE TABLE skills (
  user_id       BLOB    NOT NULL,
  name          TEXT    NOT NULL,
  version       TEXT    NOT NULL DEFAULT '0.0.0',
  source        TEXT    NOT NULL DEFAULT 'local',
  source_ref    TEXT,
  description   TEXT    NOT NULL DEFAULT '',
  enabled       INTEGER NOT NULL DEFAULT 1,
  content_hash  BLOB    NOT NULL,
  install_path  TEXT    NOT NULL,
  tool_allowlist_json TEXT NOT NULL DEFAULT '[]',
  created_at    INTEGER NOT NULL,
  updated_at    INTEGER NOT NULL,
  deleted_at    INTEGER,
  PRIMARY KEY (user_id, name),
  CHECK (name GLOB '[a-z0-9]*' AND name NOT GLOB '*[^a-z0-9-]*' AND name NOT GLOB '*..*'),
  CHECK (length(name) BETWEEN 1 AND 64),
  CHECK (source IN ('builtin','local','bundle','market')),
  CHECK (enabled IN (0,1)),
  CHECK (length(content_hash) = 32),
  FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
) STRICT;

CREATE INDEX ix_skills_enabled ON skills (user_id, enabled)
  WHERE deleted_at IS NULL;

CREATE TABLE plugins (
  user_id       BLOB    NOT NULL,
  name          TEXT    NOT NULL,
  version       TEXT    NOT NULL DEFAULT '0.0.0',
  kind          TEXT    NOT NULL,
  manifest_json TEXT    NOT NULL,
  permissions_json TEXT NOT NULL DEFAULT '[]',
  enabled       INTEGER NOT NULL DEFAULT 0,
  content_hash  BLOB    NOT NULL,
  install_path  TEXT    NOT NULL,
  created_at    INTEGER NOT NULL,
  updated_at    INTEGER NOT NULL,
  deleted_at    INTEGER,
  PRIMARY KEY (user_id, name),
  CHECK (name GLOB '[a-z0-9]*' AND name NOT GLOB '*[^a-z0-9-]*' AND name NOT GLOB '*..*'),
  CHECK (kind IN ('tool','panel','mcp_wrapper','hook')),
  CHECK (enabled IN (0,1)),
  CHECK (length(content_hash) = 32),
  FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
) STRICT;

CREATE INDEX ix_plugins_enabled ON plugins (user_id, enabled)
  WHERE deleted_at IS NULL;

-- ═══════════════ BM25 倒排索引（可重建缓存）═══════════════
--
-- ★ 这张表【就是】资料库的检索索引，BM25 在它上面执行。
--   检索流程：SQL 按 (user_id, term) 取倒排 → Rust 侧算 BM25 → 按 positions 截片段。
--   实测（verify/retrieval_final.py）：中文召回 11/15，与 FTS5+jieba 预分词持平。
--   4 个未命中均为「2 字查询词被 3 字复合词吸收」的分词边界问题（所有方案共性），
--   靠【入库端同时索引 2-gram 与复合词】解决，见 quill-wiki 的分词器约定。
--
--   真相源是 data/{uid}/wiki/wiki/**.md，本表可随时 DELETE + 重建：
--     cargo xtask wiki reindex --user <id>
--
--   两个额外索引服务两件事（均非主键前缀，故必需）：
--     ix_wiki_doc  → 增量更新时删某文档的全部词条
--     ix_wiki_path → reindex 完整性对账（xtask wiki verify 的双向 diff）
CREATE TABLE wiki_index (
  user_id      BLOB    NOT NULL,
  term         TEXT    NOT NULL,
  doc_id       BLOB    NOT NULL,
  term_kind    INTEGER NOT NULL,
  tf           INTEGER NOT NULL,
  field_len    INTEGER NOT NULL,
  positions    BLOB,
  rel_path     TEXT    NOT NULL,
  page_title   TEXT    NOT NULL,
  page_type    TEXT    NOT NULL DEFAULT '',
  content_hash BLOB    NOT NULL,
  bytes        INTEGER NOT NULL,
  indexed_at   INTEGER NOT NULL,
  PRIMARY KEY (user_id, term, doc_id, term_kind),
  CHECK (length(doc_id) = 16),
  CHECK (term_kind IN (0,1,2,3)),
  CHECK (tf > 0),
  CHECK (field_len > 0),
  CHECK (length(content_hash) = 32),
  CHECK (rel_path NOT GLOB '*..*'),
  CHECK (rel_path NOT LIKE '/%'),
  FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
) STRICT, WITHOUT ROWID;

CREATE INDEX ix_wiki_doc ON wiki_index (user_id, doc_id);
CREATE INDEX ix_wiki_path ON wiki_index (user_id, rel_path);

-- ─────────────────────────── agent 域 ───────────────────────────

CREATE TABLE sessions (
  user_id             BLOB    NOT NULL,
  id                  BLOB    NOT NULL,
  kind                TEXT    NOT NULL,
  room_id             TEXT    NOT NULL,
  team_id             BLOB,
  parent_session_id   BLOB,
  title               TEXT    NOT NULL DEFAULT '',
  expert_id           TEXT,
  provider_id         TEXT,
  model               TEXT,
  state               TEXT    NOT NULL DEFAULT 'IDLE',
  error_state         TEXT    NOT NULL DEFAULT 'none',
  last_error          TEXT,
  replan_count        INTEGER NOT NULL DEFAULT 0,
  next_seq            INTEGER NOT NULL DEFAULT 1,
  summary             TEXT,
  summary_upto_seq    INTEGER NOT NULL DEFAULT 0,
  compacted_count     INTEGER NOT NULL DEFAULT 0,
  checkpoint_key      TEXT,
  checkpoint_path     TEXT,
  workspace_path      TEXT    NOT NULL,
  message_count       INTEGER NOT NULL DEFAULT 0,
  input_tokens        INTEGER NOT NULL DEFAULT 0,
  output_tokens       INTEGER NOT NULL DEFAULT 0,
  created_at          INTEGER NOT NULL,
  updated_at          INTEGER NOT NULL,
  last_active_at      INTEGER NOT NULL,
  deleted_at          INTEGER,
  PRIMARY KEY (user_id, id),
  CHECK (length(id) = 16),
  CHECK (kind IN ('solo','team_leader','team_member')),
  CHECK (state IN ('IDLE','PLANNING','DISPATCHING','COLLECTING','DELIVERING','ABORTED','idle','running','cancelling')),
  CHECK (error_state IN ('none','interrupted','failed')),
  CHECK (length(workspace_path) BETWEEN 1 AND 512),
  CHECK (workspace_path NOT GLOB '*..*'),
  CHECK (workspace_path NOT LIKE '/%'),
  CHECK (message_count >= 0 AND next_seq >= 1),
  CHECK (summary_upto_seq >= 0 AND compacted_count >= 0),
  CHECK (kind <> 'team_member' OR (team_id IS NOT NULL AND parent_session_id IS NOT NULL AND expert_id IS NOT NULL)),
  CHECK (kind = 'solo' OR team_id IS NOT NULL),
  FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
) STRICT;

CREATE TABLE teams (
  user_id           BLOB    NOT NULL,
  id                BLOB    NOT NULL,
  name              TEXT    NOT NULL,
  room_id           TEXT    NOT NULL,
  leader_session_id BLOB    NOT NULL,
  leader_expert_id  TEXT    NOT NULL,
  guidelines        TEXT    NOT NULL DEFAULT '',
  max_dispatch      INTEGER NOT NULL DEFAULT 4,
  max_replan        INTEGER NOT NULL DEFAULT 2,
  max_ask_depth     INTEGER NOT NULL DEFAULT 3,
  state             TEXT    NOT NULL DEFAULT 'IDLE',
  state_changed_at  INTEGER NOT NULL,
  created_at        INTEGER NOT NULL,
  updated_at        INTEGER NOT NULL,
  deleted_at        INTEGER,
  PRIMARY KEY (user_id, id),
  CHECK (length(id) = 16),
  CHECK (length(name) BETWEEN 1 AND 64),
  CHECK (max_dispatch BETWEEN 2 AND 8),
  CHECK (max_replan BETWEEN 0 AND 5),
  CHECK (max_ask_depth BETWEEN 0 AND 5),
  CHECK (state IN ('IDLE','PLANNING','DISPATCHING','COLLECTING','DELIVERING','ABORTED')),
  FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE,
  FOREIGN KEY (user_id, leader_session_id) REFERENCES sessions(user_id, id)
) STRICT;

CREATE TABLE team_members (
  user_id      BLOB    NOT NULL,
  team_id      BLOB    NOT NULL,
  expert_id    TEXT    NOT NULL,
  role         TEXT    NOT NULL,
  member_session_id BLOB,
  state        TEXT    NOT NULL DEFAULT 'IDLE',
  state_changed_at INTEGER NOT NULL,
  joined_at    INTEGER NOT NULL,
  created_at   INTEGER NOT NULL,
  updated_at   INTEGER NOT NULL,
  PRIMARY KEY (user_id, team_id, expert_id),
  CHECK (role IN ('leader','member')),
  CHECK (state IN ('IDLE','PENDING','RUNNING','ASKING','DONE','FAILED','CANCELLED')),
  CHECK ((member_session_id IS NULL) = (state = 'IDLE')),
  FOREIGN KEY (user_id, team_id) REFERENCES teams(user_id, id) ON DELETE CASCADE,
  FOREIGN KEY (user_id, member_session_id) REFERENCES sessions(user_id, id)
) STRICT;

CREATE TABLE messages (
  user_id    BLOB    NOT NULL,
  id         BLOB    NOT NULL,
  session_id BLOB    NOT NULL,
  seq        INTEGER NOT NULL,
  role       TEXT    NOT NULL,
  status     TEXT    NOT NULL DEFAULT 'complete',
  content    TEXT    NOT NULL DEFAULT '',
  reasoning  TEXT,
  expert_id  TEXT,
  dispatch_id BLOB,
  tool_call_id TEXT,
  tool_name  TEXT,
  citations_json TEXT,
  input_tokens  INTEGER NOT NULL DEFAULT 0,
  output_tokens INTEGER NOT NULL DEFAULT 0,
  turn_ms    INTEGER,
  error_code TEXT,
  created_at INTEGER NOT NULL,
  PRIMARY KEY (user_id, id),
  CHECK (length(id) = 16),
  CHECK (role IN ('user','assistant','system','tool_call','tool_result')),
  CHECK (status IN ('streaming','complete','interrupted','failed')),
  CHECK (seq >= 1),
  CHECK ((role <> 'tool_call')   OR tool_call_id IS NOT NULL),
  CHECK ((role <> 'tool_result') OR tool_call_id IS NOT NULL),
  CHECK (input_tokens >= 0 AND output_tokens >= 0),
  FOREIGN KEY (user_id, session_id) REFERENCES sessions(user_id, id) ON DELETE CASCADE
) STRICT;

CREATE TABLE task_dispatches (
  user_id     BLOB    NOT NULL,
  id          BLOB    NOT NULL,
  room_id     TEXT    NOT NULL,
  team_id     BLOB    NOT NULL,
  round       INTEGER NOT NULL,
  leader_session_id BLOB NOT NULL,
  member_session_id BLOB NOT NULL,
  member_expert_id  TEXT NOT NULL,
  task_digest BLOB    NOT NULL,
  state       TEXT    NOT NULL,
  ask_depth   INTEGER NOT NULL DEFAULT 0,
  result_digest BLOB,
  result_bytes INTEGER NOT NULL DEFAULT 0,
  callback_at INTEGER,
  dispatched_at INTEGER NOT NULL,
  started_at  INTEGER,
  settled_at  INTEGER,
  deadline_at INTEGER,
  error_code  TEXT,
  error_message TEXT,
  created_at  INTEGER NOT NULL,
  updated_at  INTEGER NOT NULL,
  PRIMARY KEY (user_id, id),
  CHECK (length(id) = 16),
  CHECK (round >= 0),
  CHECK (ask_depth >= 0),
  CHECK (state IN ('PENDING','RUNNING','ASKING','DONE','FAILED','CANCELLED')),
  CHECK (result_bytes >= 0),
  CHECK ((state IN ('DONE','FAILED','CANCELLED')) = (settled_at IS NOT NULL)),
  CHECK ((state = 'ASKING') = (ask_depth > 0)),
  FOREIGN KEY (user_id, team_id) REFERENCES teams(user_id, id) ON DELETE CASCADE,
  FOREIGN KEY (user_id, member_session_id) REFERENCES sessions(user_id, id) ON DELETE CASCADE
) STRICT;



CREATE UNIQUE INDEX ux_teams_room ON teams (user_id, room_id) WHERE deleted_at IS NULL;
CREATE INDEX ix_teams_recent ON teams (user_id, updated_at DESC) WHERE deleted_at IS NULL;


CREATE INDEX ix_sessions_recent ON sessions (user_id, last_active_at DESC, id)
  WHERE deleted_at IS NULL;
CREATE INDEX ix_sessions_room ON sessions (user_id, room_id, id) WHERE deleted_at IS NULL;
CREATE UNIQUE INDEX ux_sessions_checkpoint ON sessions (user_id, checkpoint_key)
  WHERE checkpoint_key IS NOT NULL;
CREATE INDEX ix_sessions_inflight ON sessions (user_id, last_active_at)
  WHERE deleted_at IS NULL AND state IN ('PLANNING','DISPATCHING','COLLECTING','DELIVERING','running','cancelling');


CREATE UNIQUE INDEX ux_team_leader ON team_members (user_id, team_id) WHERE role = 'leader';
CREATE INDEX ix_tm_expert ON team_members (user_id, expert_id);
CREATE INDEX ix_tm_state ON team_members (user_id, team_id, state);


CREATE UNIQUE INDEX ux_messages_seq ON messages (user_id, session_id, seq);
CREATE INDEX ix_messages_context ON messages (user_id, session_id, seq DESC);
CREATE INDEX ix_messages_citations ON messages (user_id, session_id, seq)
  WHERE citations_json IS NOT NULL;
CREATE INDEX ix_messages_dispatch ON messages (user_id, dispatch_id) WHERE dispatch_id IS NOT NULL;


CREATE UNIQUE INDEX ux_dispatch_once
  ON task_dispatches (user_id, room_id, round, member_expert_id);
CREATE INDEX ix_dispatch_settle ON task_dispatches (user_id, room_id, state);
CREATE INDEX ix_dispatch_member ON task_dispatches (user_id, member_expert_id, state);
CREATE INDEX ix_dispatch_inflight ON task_dispatches (user_id, dispatched_at)
  WHERE state IN ('PENDING','RUNNING','ASKING');
