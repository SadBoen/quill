-- 定时任务（`/api/cron`）。一行 = 一个用户排期的一条投递。
--
-- **为什么是「投递一条消息」而不是「跑一个脚本」**：quill 的 Agent 能力都在对话循环里
-- （工具 / MCP / 技能 / 压缩），另写一套「任务执行器」等于把那条循环复制一份，
-- 而两套循环迟早只改一边 —— 「网页还能用、定时任务坏了」是最难发现的错法。
-- 所以调度器把 job 的 message 送进它那条会话，走的就是 `api_chat` 的
-- `prepare_turn`/`run_turn`/`finish_turn`。这条取舍与「通道」当初的一致。
--
-- **只支持两种排期**（`schedule_kind`）：
--   · `every` —— 固定间隔（秒）。纯 epoch 算术，不需要任何时区知识。
--   · `at`    —— 指定时刻（存 epoch 毫秒，写入时由 RFC3339 解析而来）。一次性：投递后软删。
-- **刻意不支持 cron 表达式**（`cron_expr` + IANA 时区）：那需要 cron 解析 + 时区库，
-- 而 DST 感知的时区算错一小时是用户可见的错。前端表单里那个选项同批去掉了 ——
-- 不画做不到的入口（与 Q041 删掉假路由、Q037 保留诚实 501 是同一条纪律）。
--
-- `next_fire_at` 是**调度器唯一要读的列**（配 `ix_cron_due` 的偏索引）：
-- 到点才被选中，所以「有没有到点」不需要扫全表。
--
-- 软删（`deleted_at`）而不是硬删：删掉的任务不该再出现，但排障时还要能看到它。
-- 一次性任务投递完也走软删 —— 记录留着，用户那条会话里有真实投递结果。
CREATE TABLE cron_jobs (
  user_id        BLOB    NOT NULL,
  id             TEXT    NOT NULL,
  name           TEXT    NOT NULL,
  message        TEXT    NOT NULL,
  schedule_kind  TEXT    NOT NULL,
  every_seconds  INTEGER,
  run_at         INTEGER,
  tz             TEXT    NOT NULL DEFAULT '',
  session_id     BLOB,
  last_fired_at  INTEGER,
  next_fire_at   INTEGER NOT NULL,
  fired_count    INTEGER NOT NULL DEFAULT 0,
  last_error     TEXT,
  created_at     INTEGER NOT NULL,
  updated_at     INTEGER NOT NULL,
  deleted_at     INTEGER,
  PRIMARY KEY (user_id, id),
  CHECK (schedule_kind IN ('every', 'at')),
  CHECK (length(name) BETWEEN 1 AND 120),
  CHECK (length(message) BETWEEN 1 AND 32000),
  -- 两个排期列与 kind 必须一一对应：既不许两个都空（没有排期），也不许两个都填（歧义）。
  CHECK ((schedule_kind = 'every') = (every_seconds IS NOT NULL)),
  CHECK ((schedule_kind = 'at') = (run_at IS NOT NULL)),
  -- 与前端表单的 min/max 同口径（60 秒 ~ 365 天）。
  CHECK (every_seconds IS NULL OR every_seconds BETWEEN 60 AND 31536000),
  CHECK (next_fire_at > 0),
  CHECK (fired_count >= 0),
  -- 与其它按用户分表的表同一条口径（0001_init 里每张都是这样）：
  -- 账号没了，它名下的定时任务不该留着继续投递。
  FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
) STRICT;

CREATE INDEX ix_cron_due ON cron_jobs (next_fire_at) WHERE deleted_at IS NULL;

CREATE INDEX ix_cron_owner ON cron_jobs (user_id, id) WHERE deleted_at IS NULL;
