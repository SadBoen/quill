-- 0007：把 mcp_servers 的契约对齐前端。
--
-- 写法上借了 Octop 的枚举命名（.octop-ref/octop/dashboard/src/api/modules/connectors.ts:128
-- 是 'stdio' | 'streamable_http'），但**不是照抄**：Octop 只有这两个值，
-- 我们多了 'sse' 与 'builtin'。所以这是「参考命名、我们自己的选择」，
-- 别把它当成逐字对齐。
--
-- 0001 建的 transport 只有 ('stdio','http','builtin')，而前端
-- ui/web/src/devices/api.ts 发的是 'stdio' | 'streamable_http' | 'sse'。
-- 两者对不上，于是**设备页的 MCP 保存按钮必然 500**（不是 501 桩，是真会炸）。
--
-- 另外三个前端字段在 0001 里根本没有对应列：
--   cwd                    stdio 进程的工作目录
--   max_concurrent_calls   服务端侧的并发上限（null = 用默认）
--   enabled_capabilities   null=全禁 / []=全开 / 数组=精确列举
--
-- SQLite 不能 ALTER 一个 CHECK 约束，只能重建表。重建时**逐列照抄** 0001 的
-- 定义（列序、类型、默认值、CHECK、外键、STRICT），漏掉一条约束就会让
-- 这张表比 0001 宽松 —— 那比枚举不匹配更难发现。
--
-- 数据搬迁：transport 原本只可能有 'stdio' / 'http'。'http' 映射为
-- 'streamable_http'（MCP 规范里 HTTP 传输现名 streamable-http，旧的
-- 'http' 是笼统叫法）。'builtin' 保留：它是 quill 内建的伪服务器，
-- 不来自外部配置。

PRAGMA foreign_keys = OFF;

CREATE TABLE mcp_servers_new (
  user_id            BLOB    NOT NULL,
  name               TEXT    NOT NULL,
  -- 与前端 McpTransport 逐字一致。抄前端是因为它已经抄了 octop，
  -- 再让数据库去迁就前端会平白多一层映射。
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
  -- 以下三列是 0001 缺的，补在这里而不是改 0001：0001 已经在真实库里跑过，
  -- 改写历史迁移会让既有数据库的迁移台账对不上。
  cwd                TEXT,
  max_concurrent_calls INTEGER,
  enabled_capabilities_json TEXT,
  asset_hash         BLOB    NOT NULL,
  created_at         INTEGER NOT NULL,
  updated_at         INTEGER NOT NULL,
  deleted_at         INTEGER,
  PRIMARY KEY (user_id, name),
  CHECK (name GLOB '[a-z0-9]*' AND name NOT GLOB '*[^a-z0-9-]*' AND name NOT GLOB '*..*'),
  CHECK (length(name) BETWEEN 1 AND 64),
  CHECK (transport IN ('stdio','streamable_http','sse','builtin')),
  CHECK (enabled IN (0,1)),
  CHECK (timeout_ms BETWEEN 1000 AND 600000),
  CHECK (length(asset_hash) = 32),
  CHECK (transport <> 'stdio' OR command IS NOT NULL),
  -- streamable_http 与 sse 都要地址；builtin 由进程内提供，没有地址。
  CHECK (transport NOT IN ('streamable_http','sse') OR url IS NOT NULL),
  -- 并发上限为 NULL 表示「用默认值」，给了就必须在 1..64。
  CHECK (max_concurrent_calls IS NULL OR max_concurrent_calls BETWEEN 1 AND 64),
  -- 三态编码：NULL=全禁，'[]'=全开，非空数组=精确列举。
  -- 只校验「是合法 JSON 且为数组」；具体有哪些能力由应用层管。
  CHECK (
    enabled_capabilities_json IS NULL
    OR (json_valid(enabled_capabilities_json)
        AND json_type(enabled_capabilities_json) = 'array')
  ),
  FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
) STRICT;

INSERT INTO mcp_servers_new (
  user_id, name, transport, command, args_json, env_json, url, headers_json,
  secret_refs_json, tool_allowlist_json, enabled, timeout_ms, description,
  asset_hash, created_at, updated_at, deleted_at
)
SELECT
  user_id, name,
  CASE transport WHEN 'http' THEN 'streamable_http' ELSE transport END,
  command, args_json, env_json, url, headers_json,
  secret_refs_json, tool_allowlist_json, enabled, timeout_ms, description,
  asset_hash, created_at, updated_at, deleted_at
FROM mcp_servers;

DROP TABLE mcp_servers;
ALTER TABLE mcp_servers_new RENAME TO mcp_servers;

CREATE INDEX ix_mcp_enabled ON mcp_servers (user_id, enabled)
  WHERE deleted_at IS NULL;

PRAGMA foreign_keys = ON;
