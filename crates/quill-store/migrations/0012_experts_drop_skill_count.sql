-- 删掉 `experts.skill_count`（queue Q103）。
--
-- **为什么是「删列」而不是「算出真值」**：`skills` 表只有 `user_id`，**没有 `expert_id`**，
-- 全仓也没有专家↔技能的关系表 —— 「这个专家有几个技能」在本 schema 里**没有答案**。
-- 这一列从建库起就只在 `experts_repo::PUT_SQL` 里被写成字面量 0，没有任何读者。
-- （别把它和**市场载荷**里同名的 `skill_count` 搞混：那是 `skillhub` 的 skillset 字段，
-- 活的、有用例覆盖，与本表这一列无关。）
--
-- **为什么是「重建表」而不是 `ALTER TABLE … DROP COLUMN`**：0001_init 里有**表级**
-- `CHECK (skill_count >= 0)`，而 SQLite 明确拒绝删掉被 CHECK 引用的列。所以走标准做法：
-- 建新表 → 整列复制 → `DROP TABLE` 旧表 → `RENAME` → 重建索引。
--
-- **列清单怎么来的**：0001 的原始列 + 0004 的 `instructions`/`model` + 0005 的
-- `source_template`（后三列由 ALTER 追加在末尾，重建时必须放在同样的位置 ——
-- 本仓一律显式列清单、不用 `SELECT *`，所以位置其实不致命，但照抄更不容易错）。
-- **CHECK 逐条照抄**，只去掉引用 `skill_count` 的那一条；0004/0005 加在列上的三条
-- CHECK 也必须一起带过来（它们是表定义的一部分）。
--
-- **数据一个字不动**：这一列从来都是 0，删它不丢任何信息。`DROP TABLE` 会连带删掉
-- 旧表的索引，所以末尾把 `ix_experts_visible` 原样重建。
CREATE TABLE experts_rebuilt (
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
  created_at     INTEGER NOT NULL,
  updated_at     INTEGER NOT NULL,
  deleted_at     INTEGER,
  instructions   TEXT    NOT NULL DEFAULT '' CHECK (length(instructions) <= 20000),
  model          TEXT    CHECK (model IS NULL OR length(trim(model)) BETWEEN 1 AND 128),
  source_template TEXT   CHECK (
    source_template IS NULL
    OR (
      length(source_template) BETWEEN 1 AND 64
      AND source_template NOT GLOB '*[^a-z0-9-]*'
    )
  ),
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
  CHECK ((is_builtin = 1) = (owner_user_id = x'00000000000000000000000000000000'))
) STRICT;

INSERT INTO experts_rebuilt (
  id, owner_user_id, display_name, version, description, role_summary, visibility,
  tool_policy_json, tags_json, license, default_enabled, is_builtin, asset_hash,
  persona_hash, created_at, updated_at, deleted_at, instructions, model, source_template
)
SELECT
  id, owner_user_id, display_name, version, description, role_summary, visibility,
  tool_policy_json, tags_json, license, default_enabled, is_builtin, asset_hash,
  persona_hash, created_at, updated_at, deleted_at, instructions, model, source_template
FROM experts;

DROP TABLE experts;

ALTER TABLE experts_rebuilt RENAME TO experts;

CREATE INDEX ix_experts_visible ON experts (visibility, default_enabled, id)
  WHERE deleted_at IS NULL;
