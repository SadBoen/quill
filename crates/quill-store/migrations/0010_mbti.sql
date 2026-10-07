-- MBTI 测评记录。一行 = 一次测完的结果，按用户分组，按时间倒序读。
--
-- 为什么存历史而不是只存「当前类型」：Octop 只在 agents 上留一个 persona_mbti
-- 字段（.octop-ref/octop/src/octop/api/routers/mbti.py:6-8）。本项目不跟着
-- 这么做 —— 测了三次就看不出「我上次是 INFP 这次怎么变 INFJ 了」，而这恰好是
-- 人格功能里用户唯一会反复做的事。历史留着，代价只是每人几十行。
--
-- 存了 answers_json（原始作答）是刻意的：只存 code 的话，
-- 以后改题库就没法解释「上次的分是怎么算出来的」。它不是敏感数据。
--
-- dimensions_json 是算出来就定死的四根轴。它冗余于 answers，但让「看上次
-- 结果」不用重跑一遍计分 —— 题库改版后旧记录仍然按当时的算法显示。

CREATE TABLE mbti_results (
  id             BLOB    NOT NULL PRIMARY KEY,
  owner_user_id  BLOB    NOT NULL,

  -- 四位 code，如 'INTJ'。没有外键指向档案表：档案是编译期常量，
  -- 加一个 CHECK 引用它做不到，写在这里的 CHECK 只能挡长度与字符集。
  code           TEXT    NOT NULL,

  -- {"ei":["I",78], "sn":["N",82], "tf":["T",75], "jp":["J",72]}
  dimensions_json TEXT   NOT NULL DEFAULT '{}',

  -- {"1":"A","2":"B",...} 原始作答
  answers_json   TEXT    NOT NULL DEFAULT '{}',

  -- 「把这个人格应用到哪个专家」。为空表示只测了没应用。
  applied_expert_id TEXT,

  -- created_at 是 unix epoch 毫秒，与 channels.created_at 同口径。
  created_at     INTEGER NOT NULL DEFAULT 0,

  CHECK (length(id) = 16),
  CHECK (length(code) = 4),
  CHECK (code = upper(code)),
  CHECK (json_valid(dimensions_json)),
  CHECK (json_valid(answers_json)),
  CHECK (length(dimensions_json) <= 1024),
  -- 28 题、每题一个字母，撑死几百字节。留 4K 够写两遍余量。
  CHECK (length(answers_json) <= 4096),
  CHECK (applied_expert_id IS NULL OR length(applied_expert_id) BETWEEN 1 AND 64),
  CHECK (created_at >= 0),
  CHECK (owner_user_id IS NOT NULL)
) STRICT;

-- 「当前类型」= 这个用户最新一条。倒着排，LIMIT 1 就是。
CREATE INDEX ix_mbti_results_owner ON mbti_results (owner_user_id, created_at DESC);