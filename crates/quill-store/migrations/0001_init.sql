
CREATE TABLE schema_version (
  version     INTEGER NOT NULL PRIMARY KEY,
  name        TEXT    NOT NULL,
  checksum    BLOB    NOT NULL,
  applied_at  INTEGER NOT NULL,
  exec_ms     INTEGER NOT NULL,
  app_version TEXT    NOT NULL,
  note        TEXT    NOT NULL DEFAULT ''
) STRICT;

CREATE TABLE users (
  id               BLOB    NOT NULL PRIMARY KEY,
  username         TEXT    NOT NULL,
  username_norm    TEXT    NOT NULL,
  display_name     TEXT    NOT NULL,
  password_hash    BLOB    NOT NULL,
  password_salt    BLOB    NOT NULL,

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

  CHECK (expires_at > created_at),
  FOREIGN KEY (created_by) REFERENCES users(id) ON DELETE CASCADE,
  FOREIGN KEY (accepted_by) REFERENCES users(id) ON DELETE SET NULL
) STRICT;

CREATE UNIQUE INDEX ux_invites_code ON invites (code_hash);

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
