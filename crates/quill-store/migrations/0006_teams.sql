-- 0006 专家团 CRUD。
--
-- teams / team_members 两张表在 0001 就已建好，session 与派工两处外键
-- （sessions.team_id、task_dispatches.team_id）都指向 teams(user_id, id)。
-- 因此这里**不重建表**，只补 CRUD 契约缺的两列：重建会连带打断那两条外键，
-- 属于「为了让一个路由好写而破坏已接通路由」。

-- 公开标识。0001 的主键是 16 字节 BLOB（派工路由按 32 位 hex 解析 {id}），
-- 而对外契约要求 team_id 是 kebab-case 字符串。两者共存：
-- BLOB 只当内部外键键，team_slug 才是用户可见、可读、可记的标识，
-- 解析口径与 experts.id 完全一致（quill_adapters::ids::validate_slug）。
ALTER TABLE teams ADD COLUMN team_slug TEXT NOT NULL DEFAULT '';
ALTER TABLE teams ADD COLUMN description TEXT;

-- 老行回填一个合法且可读的 slug：team- 前缀 + 32 位小写 hex（长度 37，在
-- slug 长度上限 64 以内）。不回填的话这些行对 CRUD 接口等于不存在。
UPDATE teams SET team_slug = 'team-' || lower(hex(id)) WHERE team_slug = '';

-- 公开标识在同一用户下唯一，但只约束未软删的行：与 experts「软删后同名可
-- 重建」的口径一致，否则删过的团队号会永久变成死号。
CREATE UNIQUE INDEX ux_teams_slug
  ON teams (user_id, team_slug) WHERE deleted_at IS NULL AND team_slug <> '';

-- 幂等删除要能按 slug 找到「已软删的那一行」，部分唯一索引覆盖不到软删行，
-- 所以补一个不带 WHERE 的查询索引。
CREATE INDEX ix_teams_slug_lookup ON teams (user_id, team_slug);

-- 成员唯一性由 0001 的 PRIMARY KEY (user_id, team_id, expert_id) 保证：
-- 同一专家在同一团队里写两次会直接撞主键，而不是静默多出一行。
-- 这条索引只是给「某专家加入了哪些团队」这类巡检查询用（ix_tm_expert 已覆盖
-- 同一件事，故不重复建）。
