-- 多模型供应商：模型管理页的数据源（预设提供商 / 自定义端点）。
--
-- id 是 32 位大写 hex 的 TEXT（与项目既有约定一致：写路径用大写 hex，
-- 因为 SQLite hex() 读出来也是大写，两边必须逐字节相同）。
-- kind 区分「预设」与「自定义」，protocol 只允许 openai / anthropic；
-- admin_config 里历史存在的 openrouter 落到本表时映射成 openai。
-- token 上限的跨字段约束与 0002 同口径：context > 0；compaction > 4000
-- 且 <= context；output > 0 且 <= context。这些约束只在这里有，
-- 「绕过 API 直写 INSERT」这条路径上也仍然拒绝坏数据。

CREATE TABLE llm_providers (
  id                            TEXT    PRIMARY KEY,
  name                          TEXT    NOT NULL,
  preset_id                     TEXT    NOT NULL,
  kind                          TEXT    NOT NULL CHECK (kind IN ('preset','custom')),
  protocol                      TEXT    NOT NULL CHECK (protocol IN ('openai','anthropic')),
  base_url                      TEXT    NOT NULL,
  api_key                       TEXT    NOT NULL DEFAULT '',
  model                         TEXT    NOT NULL DEFAULT '',
  max_context_tokens            INTEGER NOT NULL,
  compaction_threshold_tokens   INTEGER NOT NULL,
  max_output_tokens             INTEGER NOT NULL,
  enabled                       INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0,1)),
  is_default                    INTEGER NOT NULL DEFAULT 0 CHECK (is_default IN (0,1)),
  created_at                    INTEGER NOT NULL,
  updated_at                    INTEGER NOT NULL,
  CHECK (length(name)      BETWEEN 1 AND 128),
  CHECK (length(preset_id) BETWEEN 1 AND 64),
  CHECK (length(base_url)  BETWEEN 1 AND 512),
  CHECK (length(model)     <= 256),
  CHECK (length(api_key)   <= 4096),
  CHECK (max_context_tokens          > 0),
  CHECK (compaction_threshold_tokens > 4000),
  CHECK (compaction_threshold_tokens <= max_context_tokens),
  CHECK (max_output_tokens           > 0),
  CHECK (max_output_tokens           <= max_context_tokens),
  CHECK (created_at >= 0),
  CHECK (updated_at >= 0)
) STRICT;
