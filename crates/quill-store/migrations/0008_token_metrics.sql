-- 0008：给 messages 补上 goose 口径的缓存 token 列，为会话级 Token 统计条打底。
--
-- 为什么需要：quill 原本只存 input_tokens / output_tokens 两列。而 goose 的
-- `Usage`（vendor/goose/crates/goose-provider-types/src/conversation/token_usage.rs:93）
-- 有五个字段：
--
--   input_tokens               含缓存读写的**全部**入参
--   output_tokens
--   total_tokens
--   cache_read_input_tokens    input 的子集
--   cache_write_input_tokens   input 的子集
--
-- 缓存两列缺了，「缓存命中」这类指标就只能靠猜 —— 而猜出来的数字比没有数字
-- 更坏：用户会拿它做决策。所以这里把两列补上，让统计条只显示真实上报的值。
--
-- 语义（照抄 goose 的文档注释，别自行发挥）：
--   - cache_read/cache_write 是 input 的**子集**，不要重复计入 total。
--   - 这两列**可空，而且是必须可空**：NULL = 上游没上报（不知道），
--     0 = 上游报了确实是 0。多数本地模型压根不报缓存 token，
--     拿 0 冒充「已上报且命中为零」，界面就会显示一个凭空捏造的
--     「命中率 0%」—— 那是本项目最不该出现的一类数字。
--     goose 的 `Usage` 用 `Option<i32>` 表达同一件事，这里跟着它。
--     input_tokens/output_tokens 保持 0001 的 NOT NULL DEFAULT 0 不动：
--     那两列 0 是有意义的真实值（上游确实报了 0）。
--
-- **为什么只改 messages，不改 sessions**
--
-- sessions 上早就有 input_tokens / output_tokens 两个滚动汇总列。这里刻意
-- 不给它加缓存列：
--   - 汇总值应当是能由明细复算出来的派生量（见 session_metrics.rs），
--     再存一份就多一个会漂移的真相来源。
--   - NULL 的累加语义是陷阱：SQL 里 `NULL + 5` 仍是 NULL，所以要么让列
--     永远停在 NULL、要么得写一堆 CASE WHEN —— 而这两种行为都得写注释解释，
--     正说明那个字段本来就不该存在。
--   - 统计接口直接从 messages 聚合，实测会话只有几十条，一次聚合不贵。
--
-- 改表方式：SQLite 不能 ALTER 出一个带 CHECK 的新列，只能重建表。重建时
-- **逐列照抄** 0001 的定义（列序、类型、默认值、CHECK、外键、STRICT），漏掉
-- 一条约束就会让这张表比 0001 宽松 —— 那比枚举不匹配更难发现。

PRAGMA foreign_keys = OFF;

CREATE TABLE messages_new (
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
  -- 0008 新增：这一条 assistant 消息自己上报的缓存拆分项。
  -- 单轮明细是会话级聚合的原料 —— 会话级数字必须能由它复算出来。
  cache_read_tokens  INTEGER,
  cache_write_tokens INTEGER,
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
  CHECK (cache_read_tokens IS NULL OR cache_read_tokens >= 0),
  CHECK (cache_write_tokens IS NULL OR cache_write_tokens >= 0),
  FOREIGN KEY (user_id, session_id) REFERENCES sessions(user_id, id) ON DELETE CASCADE
) STRICT;

-- 老行两列一律给 NULL：0008 之前 quill 压根没读过缓存 token，
-- 那些行的 0 是「没记过」而不是「记过且为零」。
INSERT INTO messages_new (
  user_id, id, session_id, seq, role, status, content, reasoning, expert_id,
  dispatch_id, tool_call_id, tool_name, citations_json, input_tokens,
  output_tokens, cache_read_tokens, cache_write_tokens, turn_ms, error_code,
  created_at
)
SELECT
  user_id, id, session_id, seq, role, status, content, reasoning, expert_id,
  dispatch_id, tool_call_id, tool_name, citations_json, input_tokens,
  output_tokens, NULL, NULL, turn_ms, error_code, created_at
FROM messages;

DROP TABLE messages;
ALTER TABLE messages_new RENAME TO messages;

-- DROP TABLE 会连索引一起带走，四条都要照 0001 原样重建。
CREATE UNIQUE INDEX ux_messages_seq ON messages (user_id, session_id, seq);
CREATE INDEX ix_messages_context ON messages (user_id, session_id, seq DESC);
CREATE INDEX ix_messages_citations ON messages (user_id, session_id, seq)
  WHERE citations_json IS NOT NULL;
CREATE INDEX ix_messages_dispatch ON messages (user_id, dispatch_id) WHERE dispatch_id IS NOT NULL;

PRAGMA foreign_keys = ON;