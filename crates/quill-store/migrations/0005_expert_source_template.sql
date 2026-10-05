-- 专家的「来源模板」回指列：记录这个专家是从哪个预设模板派生出来的。
-- 同一条记录必须是 1:N 的 —— 一个模板可以生成任意多个专家（例如从
-- ai-coding-coach 派生「程序员1号」「程序员2号」），所以这一列存的是
-- 模板 id 本身，不存专家 id，也不加唯一约束。
--
-- 取舍：不做存在性校验（没有外键、不查模板表）。前端模板库是静态 vendor
-- 进来的，后端没有对应的表可查；真要外键就得先建一张模板表，那是另一件事。
--
-- 允许 NULL 是必须的：NULL = 这个专家不是从模板生成的（手建 / 内置专家）。
-- 用空串表达同一件事会让「没有来源」和「来源被清空」在 SQL 里分不开，
-- 而 PATCH 的省略 / 显式 null 两种语义正需要这一个区分。
--
-- 只加列：0001 的 role_summary / persona_hash 与 0004 的 instructions /
-- model 语义一律不动。

ALTER TABLE experts ADD COLUMN source_template TEXT
  CHECK (
    source_template IS NULL
    OR (
      length(source_template) BETWEEN 1 AND 64
      AND source_template NOT GLOB '*[^a-z0-9-]*'
    )
  );
