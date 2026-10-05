-- 专家人格四件套里的后两件：instructions（人格正文）与 model（偏好模型）。
-- 字段口径照抄 vendor/goose 的 custom agent：一个专家 = 带 frontmatter 的
-- markdown，frontmatter 的 name/description 对应本表已有的 id/display_name/
-- description，markdown 正文对应 instructions，frontmatter 的 model 对应 model。
-- model 为 NULL 表示跟随实例默认模型（goose 里 frontmatter 不写 model 同义）。
--
-- 只加列：0001 已有的 role_summary / persona_hash 语义一律不动。

ALTER TABLE experts ADD COLUMN instructions TEXT NOT NULL DEFAULT ''
  CHECK (length(instructions) <= 20000);

ALTER TABLE experts ADD COLUMN model TEXT
  CHECK (model IS NULL OR length(trim(model)) BETWEEN 1 AND 128);
