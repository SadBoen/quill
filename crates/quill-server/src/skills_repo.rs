//! SKILL：把一段可复用的做法存成可被模型调用的东西。
//!
//! **抄 Octop 的模型**（`dashboard/src/api/types/skill.ts`）：
//! `SkillListItem` 里有 `tool_name` —— 在 Octop 里 **SKILL 就是一个工具**。
//! 这个选择是对的，因为它让「SKILL 怎么进 prompt」这个问题**不必回答**：
//! 它不走 prompt，走 `tools` 字段，由模型自己决定何时调用。
//!
//! 与 Octop 的差别（不是省略，是 quill 没有的东西）：
//! - Octop 有 `skillhub` 市场、bundle 导入导出、多端同步；quill 只有本地的。
//! - Octop 的 `SkillDetail.content` 来自磁盘上的 `skills/{slug}/SKILL.md`；
//!   quill 存在数据库里（`skills` 表 0001 就建好了，`content_hash` 是必填的
//!   16 字节列，说明设计时就打算存内容摘要）。
//!
//! **安全**：SKILL 内容会被当作工具描述发给模型。写入时要限长 ——
//! 一段几万字的「技巧」会把请求体撑爆，而且模型也读不完。

use serde_json::{json, Value};
use sqlx::sqlite::SqliteRow;
use sqlx::Row;

use quill_adapters::UserId;

use crate::db::{digest32, now_ms, storage_error, DbBridge};

const OP_LIST: &str = "列出 SKILL";
const OP_WRITE: &str = "写入 SKILL";

/// 单个 SKILL 的正文上限。
///
/// 参照 `crates/quill-agent/src/expert.rs` 的 `MAX_INSTRUCTIONS_CHARS`
/// （20000）取同一个量级：专家人格已经用掉 20000，再多会让上下文迅速膨胀。
/// 超了就在这里报 400，而不是截断 —— 截断后的 SKILL 是个半截东西，
/// 用户以为装上了，模型按残缺内容行事，比报错难查得多。
pub const MAX_SKILL_CHARS: usize = 20_000;

pub const COLUMNS: &str = "name, version, source, source_ref, description, enabled, \
     content_hash, install_path, tool_allowlist_json, created_at, updated_at";

pub const LIST_SQL: &str = "SELECT name, version, source, source_ref, description, enabled, \
     content_hash, install_path, tool_allowlist_json, created_at, updated_at FROM skills \
     WHERE user_id = ? AND deleted_at IS NULL ORDER BY name ASC";

pub const GET_SQL: &str = "SELECT name, version, source, source_ref, description, enabled, \
     content_hash, install_path, tool_allowlist_json, created_at, updated_at FROM skills \
     WHERE user_id = ? AND name = ? AND deleted_at IS NULL";

#[derive(Debug, Clone, PartialEq)]
pub struct SkillRow {
    pub name: String,
    pub version: String,
    /// `builtin` | `local` | `bundle` | `market`（CHECK 允许的四个）。
    pub source: String,
    pub source_ref: Option<String>,
    pub description: String,
    pub enabled: bool,
    pub install_path: String,
    pub tool_allowlist: Vec<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// SKILL 正文。正文不进 `skills` 行（表里没有这一列），另存一份到
/// `QUILL_SKILL_DIR`，`skills` 行只留摘要与路径 —— 与 Octop 的
/// 「挂目录给运行时」一致，也让 `content_hash` 名副其实。
pub struct SkillWithContent {
    pub row: SkillRow,
    pub content: String,
}

/// 内容摘要。前端据此判断「装的是不是同一份」。
///
/// 32 字节是 `skills.content_hash` 的 CHECK 硬要求（`length(content_hash) = 32`），
/// 所以复用 `db::digest32` 而不是随手截一段 SHA-256：截短了编译得过、单测也过，
/// 但每一行写入都会被数据库拒掉。
pub fn content_hash(content: &str) -> Vec<u8> {
    digest32("skills.content_hash", &[content.as_bytes()]).to_vec()
}

pub async fn list(db: &DbBridge, uid: UserId) -> Result<Vec<SkillRow>, quill_agent::AgentError> {
    let b = crate::db::blob_of(&uid);
    db.call(move |pool, _rt| {
        Box::pin(async move {
            let rows = sqlx::query(LIST_SQL)
                .bind(&b)
                .fetch_all(&pool)
                .await
                .map_err(|e| storage_error(OP_LIST, e))?;
            rows.iter().map(row_from).collect()
        })
    })
}

pub async fn get(
    db: &DbBridge,
    uid: UserId,
    name: &str,
) -> Result<Option<SkillRow>, quill_agent::AgentError> {
    let b = crate::db::blob_of(&uid);
    let n = name.to_string();
    db.call(move |pool, _rt| {
        Box::pin(async move {
            sqlx::query(GET_SQL)
                .bind(&b)
                .bind(&n)
                .fetch_optional(&pool)
                .await
                .map_err(|e| storage_error(OP_LIST, e))?
                .as_ref()
                .map(row_from)
                .transpose()
        })
    })
}

/// 写入或更新一个 SKILL。目录侧的正文由调用方（`api_extensions`）落盘，
/// 这里只管行。
#[allow(clippy::too_many_arguments)]
pub async fn upsert(
    db: &DbBridge,
    uid: UserId,
    row: SkillRow,
    hash: Vec<u8>,
) -> Result<SkillRow, quill_agent::AgentError> {
    let b = crate::db::blob_of(&uid);
    let allow = serde_json::to_string(&row.tool_allowlist)
        .map_err(|e| storage_error(OP_WRITE, e))?;
    db.call(move |pool, _rt| {
        Box::pin(async move {
            let now = now_ms();
            let r = sqlx::query(
                "INSERT INTO skills (user_id, name, version, source, source_ref, description, \
                 enabled, content_hash, install_path, tool_allowlist_json, created_at, updated_at, \
                 deleted_at) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,NULL) \
                 ON CONFLICT(user_id, name) DO UPDATE SET \
                   version=excluded.version, source=excluded.source, \
                   source_ref=excluded.source_ref, description=excluded.description, \
                   enabled=excluded.enabled, content_hash=excluded.content_hash, \
                   install_path=excluded.install_path, \
                   tool_allowlist_json=excluded.tool_allowlist_json, \
                   updated_at=excluded.updated_at, deleted_at=NULL",
            )
            .bind(&b)
            .bind(&row.name)
            .bind(&row.version)
            .bind(&row.source)
            .bind(&row.source_ref)
            .bind(&row.description)
            .bind(if row.enabled { 1i64 } else { 0i64 })
            .bind(&hash)
            .bind(&row.install_path)
            .bind(&allow)
            .bind(now)
            .bind(now)
            .execute(&pool)
            .await
            .map_err(|e| storage_error(OP_WRITE, e))?;
            if r.rows_affected() == 0 {
                return Err(storage_error(
                    OP_WRITE,
                    std::io::Error::other("写入未影响任何行"),
                ));
            }
            // 回读而不是把 row 原样返回：落库时库会填默认值（如 version 归一），
            // 拿「我们以为写进去的样子」当结果，前端存完立刻 GET 就会对不上。
            let read = sqlx::query(GET_SQL)
                .bind(&b)
                .bind(&row.name)
                .fetch_optional(&pool)
                .await
                .map_err(|e| storage_error(OP_WRITE, e))?;
            match read.as_ref().map(row_from).transpose()? {
                Some(v) => Ok(v),
                None => Err(storage_error(
                    OP_WRITE,
                    format!("刚写入的 SKILL {:?} 立刻就读不到了", row.name),
                )),
            }
        })
    })
}

/// 只改启用开关，**一个别的列都不碰**。
///
/// ## 为什么不能复用 [`upsert`]
///
/// `upsert` 收一个 `hash: Vec<u8>` 参数，而这个值是**直接写进
/// `skills.content_hash` 列**的。拿它做「只改 enabled」时，调用方手里
/// 并没有真正的正文哈希 —— 随手传一个（哪怕是 `""` 的哈希）就会把
/// 这一列悄悄改成一个与磁盘正文对不上的值，而且**没有任何报错**：
/// 哈希只在被用来比对时才露馅，那时已经很难追了。
///
/// 另一条同源的坑：`upsert` 还会把 `deleted_at` 重置成 NULL。
/// 只想启用一个技能却顺手把它「复活」了，也是它。
///
/// 所以这里是**一条只写 `enabled` 的 UPDATE**。它没有别的列可写，
/// 结构上就不可能改坏别的东西 —— 这一点比在这里写注释提醒调用方可靠。
pub async fn set_enabled(
    db: &DbBridge,
    uid: UserId,
    name: &str,
    enabled: bool,
) -> Result<bool, quill_agent::AgentError> {
    let b = crate::db::blob_of(&uid);
    let n = name.to_string();
    db.call(move |pool, _rt| {
        Box::pin(async move {
            let r = sqlx::query(
                "UPDATE skills SET enabled = ?, updated_at = ? \
                 WHERE user_id = ? AND name = ? AND deleted_at IS NULL",
            )
            .bind(if enabled { 1i64 } else { 0i64 })
            .bind(now_ms())
            .bind(&b)
            .bind(&n)
            .execute(&pool)
            .await
            .map_err(|e| storage_error(OP_WRITE, e))?;
            Ok(r.rows_affected() > 0)
        })
    })
}

/// 软删。返回是否真的删掉了 —— 已经不在了返回 false，让 DELETE 幂等。
pub async fn soft_delete(
    db: &DbBridge,
    uid: UserId,
    name: &str,
) -> Result<bool, quill_agent::AgentError> {
    let b = crate::db::blob_of(&uid);
    let n = name.to_string();
    db.call(move |pool, _rt| {
        Box::pin(async move {
            let now = now_ms();
            let r = sqlx::query(
                "UPDATE skills SET deleted_at = ?, updated_at = ? \
                 WHERE user_id = ? AND name = ? AND deleted_at IS NULL",
            )
            .bind(now)
            .bind(now)
            .bind(&b)
            .bind(&n)
            .execute(&pool)
            .await
            .map_err(|e| storage_error(OP_WRITE, e))?;
            Ok(r.rows_affected() > 0)
        })
    })
}

fn s(row: &SqliteRow, k: &str) -> Result<String, quill_agent::AgentError> {
    row.try_get::<String, _>(k)
        .map_err(|e| storage_error(OP_LIST, e))
}

fn os(row: &SqliteRow, k: &str) -> Result<Option<String>, quill_agent::AgentError> {
    row.try_get::<Option<String>, _>(k)
        .map_err(|e| storage_error(OP_LIST, e))
}

fn row_from(r: &SqliteRow) -> Result<SkillRow, quill_agent::AgentError> {
    let allow_raw = s(r, "tool_allowlist_json")?;
    let tool_allowlist = serde_json::from_str::<Vec<String>>(&allow_raw).map_err(|e| {
        storage_error(
            &format!("{OP_LIST}（tool_allowlist_json 列的内容不是合法 JSON 数组）"),
            e,
        )
    })?;
    Ok(SkillRow {
        name: s(r, "name")?,
        version: s(r, "version")?,
        source: s(r, "source")?,
        source_ref: os(r, "source_ref")?,
        description: s(r, "description")?,
        enabled: r
            .try_get::<i64, _>("enabled")
            .map_err(|e| storage_error(OP_LIST, e))?
            != 0,
        install_path: s(r, "install_path")?,
        tool_allowlist,
        created_at: r
            .try_get::<i64, _>("created_at")
            .map_err(|e| storage_error(OP_LIST, e))?,
        updated_at: r
            .try_get::<i64, _>("updated_at")
            .map_err(|e| storage_error(OP_LIST, e))?,
    })
}

/// 转成 Octop 的 `SkillListItem` 形状。`tool_name` 沿用 Octop 约定：
/// SKILL 在模型眼里就是一个工具，工具名即 slug。
pub fn to_json(r: &SkillRow) -> Value {
    json!({
        "slug": r.name,
        "tool_name": r.name,
        "description": r.description,
        "version": r.version,
        "source": r.source,
        "kind": if r.source == "builtin" { "builtin" } else { "workspace" },
        "enabled": r.enabled,
        "path": r.install_path,
        "tool_allowlist": r.tool_allowlist,
    })
}

/// 工具描述里最多放多少字符的技能摘要。
///
/// **这个数字是算出来的，不是拍的**：实测一个 8192 上下文的模型上，
/// 13 个启用技能的正文合计 32430 字符（≈10810 tokens），**光固定开销就已经
/// 超窗**，于是连「1+1 等于几」都必然 503。见 ISSUE-036。
/// 描述改成有界摘要之后，同样的 13 个技能常驻开销降到 3k 字符量级。
pub const MAX_SKILL_DESCRIPTION_CHARS: usize = 240;

/// 从正文开头取一段有界摘要，给「frontmatter 的 description 为空」的情况兜底。
///
/// 仍然要有摘要：模型得先知道这个技能**管不管用得上**，才会决定调不调它。
/// 完全没有描述的话，模型就只能靠工具名瞎猜。
fn body_summary(content: &str) -> String {
    let mut out: String = content.chars().take(MAX_SKILL_DESCRIPTION_CHARS).collect();
    if content.chars().count() > MAX_SKILL_DESCRIPTION_CHARS {
        out.push_str("……（完整方法正文在调用该技能时回灌，这里只是摘要。）");
    }
    out
}

/// 工具描述用的摘要。
///
/// 优先用 SKILL 自己 frontmatter 里的 `description` —— 那本来就是给它看的；
/// 没有就退回正文开头一段。
pub fn skill_summary(r: &SkillRow, content: &str) -> String {
    let desc = r.description.trim();
    if !desc.is_empty() {
        let mut out: String = desc.chars().take(MAX_SKILL_DESCRIPTION_CHARS).collect();
        if desc.chars().count() > MAX_SKILL_DESCRIPTION_CHARS {
            out.push_str("……（完整方法正文在调用该技能时回灌，这里只是摘要。）");
        }
        return out;
    }
    body_summary(content)
}

/// 把 SKILL 变成一个 `ToolSpec`，挂进 `ToolRegistry`。
///
/// 这就是「SKILL 即工具」的落地点：模型自己决定何时调用这个技能。
///
/// **描述里只放摘要，不放整段正文。** 正文在技能被调用时才回灌。
/// 之前是把正文整段塞进 `description` 的，注释还写着「SKILL 不占常驻上下文
/// ——用不到就完全不出现」——**那句话是反的**：工具描述每轮请求都带着，
/// 所以每个启用技能的全文都是**常驻**开销，用户装十几个技能就会把上下文窗口
/// 顶爆，而界面一个字都不说。见 ISSUE-036。
pub fn as_tool_spec(r: &SkillRow, content: &str) -> quill_provider::ToolSpec {
    quill_provider::ToolSpec::new(&r.name, skill_summary(r, content)).with_parameters(json!({
        "type": "object",
        "properties": {
            "task": {
                "type": "string",
                "description": "要交给这套方法处理的具体任务描述。"
            }
        },
        "required": ["task"]
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(name: &str) -> SkillRow {
        SkillRow {
            name: name.to_string(),
            version: "0.1.0".into(),
            source: "local".into(),
            source_ref: None,
            description: "示例".into(),
            enabled: true,
            install_path: "/tmp/x".into(),
            tool_allowlist: vec![],
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn the_list_sql_filters_by_user_and_excludes_soft_deleted() {
        assert!(LIST_SQL.contains("user_id = ?"));
        assert!(LIST_SQL.contains("deleted_at IS NULL"));
    }

    #[test]
    fn the_tool_description_carries_a_summary_not_the_whole_body() {
        // 修之前 `ToolSpec::new(name, content)` 把整段正文塞进 description，
        // 而工具描述每轮请求都带着 —— 13 个技能就是 32430 字符（≈10810 tokens），
        // 光固定开销就超了 8192 的窗口。见 ISSUE-036。
        let body = "行".repeat(5_000);
        let mut r = row("mesh-analysis");
        // frontmatter 的描述写得足够长，正好逼出截断标记
        r.description = "描".repeat(MAX_SKILL_DESCRIPTION_CHARS + 100);
        let spec = as_tool_spec(&r, &body);
        let desc = &spec.description;
        assert!(
            desc.chars().count() <= MAX_SKILL_DESCRIPTION_CHARS + 80,
            "描述必须有界，实测 {} 字符",
            desc.chars().count()
        );
        assert!(
            !desc.contains(&"行".repeat(1_000)),
            "正文不该整段出现在描述里"
        );
        assert!(
            desc.contains("调用该技能时回灌"),
            "要说清摘要在哪看全集：{desc}"
        );
    }

    #[test]
    fn a_short_frontmatter_description_is_used_verbatim() {
        // 没被截断时不要硬塞「回灌」标记，否则每条描述都多一句废话。
        let spec = as_tool_spec(&row("csv-processing"), &"行".repeat(5_000));
        assert_eq!(spec.description, "示例");
    }

    #[test]
    fn a_skill_without_a_frontmatter_description_falls_back_to_the_head_of_the_body() {
        // 描述为空时模型就只能靠工具名瞎猜，所以从正文开头取一段。
        let mut r = row("no-desc");
        r.description = "   ".into();
        let body = format!("第一步：{}{}", "甲", "乙".repeat(2_000));
        let spec = as_tool_spec(&r, &body);
        assert!(spec.description.contains("第一步：甲"));
        assert!(
            spec.description.chars().count() <= MAX_SKILL_DESCRIPTION_CHARS + 80,
            "兜底摘要同样必须有界：{} 字符",
            spec.description.chars().count()
        );
    }

    #[test]
    fn the_summary_limit_is_small_enough_that_thirteen_skills_fit_an_8k_window() {
        // 这条断言是 ISSUE-036 的直接回归：13 * (240 + 60) ≈ 3900 字符，
        // 加上人格与工具表也还在 8192 以内。数字变了就说明预算又松了。
        let thirteen = 13 * (MAX_SKILL_DESCRIPTION_CHARS + 60);
        assert!(
            thirteen < 4_000,
            "13 个技能的常驻描述预算 {thirteen} 字符，超出可接受范围"
        );
    }

    #[test]
    fn the_octop_shape_keeps_tool_name_equal_to_slug() {
        // Octop 的 SkillListItem 就有 tool_name；它是「SKILL 即工具」这个
        // 约定的载体，去掉就等于改了语义。
        let j = to_json(&row("code-review"));
        assert_eq!(j["tool_name"], json!("code-review"));
        assert_eq!(j["slug"], json!("code-review"));
        assert_eq!(j["kind"], json!("workspace"));
        assert_eq!(to_json(&row("x")).get("tool_name").is_some(), true);
    }

    #[test]
    fn a_builtin_skill_reports_kind_builtin() {
        let mut r = row("b");
        r.source = "builtin".into();
        assert_eq!(to_json(&r)["kind"], json!("builtin"));
    }

    #[test]
    fn the_content_hash_is_the_length_the_schema_demands() {
        // content_hash 的 CHECK 是 length = 32。
        assert_eq!(content_hash("任意内容").len(), 32);
    }

    #[test]
    fn the_hash_changes_with_content_and_is_stable_otherwise() {
        assert_eq!(content_hash("a"), content_hash("a"));
        assert_ne!(content_hash("a"), content_hash("b"));
    }

    #[test]
    fn a_skill_becomes_a_tool_with_a_requiring_task_argument() {
        let spec = as_tool_spec(&row("code-review"), "如何审代码：先看 diff……");
        assert_eq!(spec.name, "code-review");
        // 描述取 frontmatter 的 description（「示例」），不再塞正文 —— 见 ISSUE-036。
        assert_eq!(spec.description, "示例");
        assert_eq!(
            spec.parameters["required"],
            json!(["task"]),
            "没有必填参数的话模型会不知道该传什么"
        );
    }

    #[test]
    fn the_size_limit_is_the_same_order_as_the_expert_persona_limit() {
        // 两处上限应同量级，否则会出现「专家人格放得下、SKILL 放不下」
        // 这种说不清的失败。
        assert_eq!(MAX_SKILL_CHARS, 20_000);
        assert!(MAX_SKILL_CHARS <= 20_000);
    }
}
