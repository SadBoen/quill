-- 实例级 LLM provider 配置，单行单实例。
-- 单行用 `id = 1` 强制约束，写入时 `INSERT OR REPLACE` 也只命中这一行；
-- 字段都是 NOT NULL，避免 NULL 当成"未设"的歧义。
-- protocol 是 enum：openai（chat completions 兼容）/ anthropic（Messages）/ openrouter。
-- token 上限必须 > 0；max_context_tokens 还要 >= compaction_threshold_tokens
-- 且 compaction_threshold_tokens 要 > 4000（推理模型预算下限，避免
-- 一压就空，留出正文空间）。这些跨字段约束只在这里有。

CREATE TABLE admin_config (
  id                            INTEGER PRIMARY KEY,
  protocol                      TEXT    NOT NULL,
  base_url                      TEXT    NOT NULL,
  api_key                       TEXT    NOT NULL DEFAULT '',
  model                         TEXT    NOT NULL,
  max_context_tokens            INTEGER NOT NULL,
  compaction_threshold_tokens   INTEGER NOT NULL,
  max_output_tokens             INTEGER NOT NULL,
  updated_at                    INTEGER NOT NULL,
  CHECK (id = 1),
  CHECK (protocol IN ('openai','anthropic','openrouter')),
  CHECK (length(protocol) BETWEEN 1 AND 32),
  CHECK (length(base_url) BETWEEN 1 AND 512),
  CHECK (length(model)    BETWEEN 1 AND 128),
  CHECK (max_context_tokens          > 0),
  CHECK (max_context_tokens          <= 1_000_000),
  CHECK (compaction_threshold_tokens > 4000),
  CHECK (max_output_tokens           > 0),
  CHECK (max_output_tokens           <= 32768),
  CHECK (compaction_threshold_tokens <= max_context_tokens),
  CHECK (max_output_tokens           <= max_context_tokens),
  CHECK (updated_at >= 0)
) STRICT;
