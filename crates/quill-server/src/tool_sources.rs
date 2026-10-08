//! `ToolSources` 的壳侧实现：把内核要的三份清单**从既有函数里查出来**。
//!
//! 内核（`quill-core`）不认识 `AppState` / `DbBridge` / 技能目录，只声明它要什么
//! （`quill_core::tools::ToolSources`，见 `docs/KERNEL-PORTS.md §2`）；这里负责
//! 「从哪拿」：
//!
//! | 端口方法 | 转发到 |
//! |---|---|
//! | `experts_for_tools` | `api_experts::list_for_tools`（与 `GET /api/experts` 同一套可见性）|
//! | `skills` | `skills_repo::list_blocking` + `api_extensions::{skill_body_path, read_skill_body}` |
//! | `mcp_servers` | `mcp_repo::list_blocking`（只给配置行；握手与 `tools/list` 仍由内核 `mcp_client` 做）|
//!
//! **行为一个字不改**：每个方法都是转发，行到行的搬运之外不做任何判断 ——
//! 可见性、停用、正文缺失这些口径全部留在原函数与内核里（在这里再判一次就是
//! 「界面说一套、模型做另一套」的第二份真相）。
//!
//! **为什么 trait 是同步的**：`ToolRegistry::builtin*` 本身就是同步构造，
//! 而 `DbBridge::call` 本来就是阻塞调用（`skills_repo::list` / `mcp_repo::list`
//! 的 async 只是外壳）—— 所以同步函数是真正的实现，`*_blocking` 就是它。

use std::path::PathBuf;
use std::sync::Arc;

use quill_adapters::UserId;
use quill_core::mcp::McpServerRow;
use quill_core::tools::{ExpertToolRow, SkillToolRow, ToolSourceError, ToolSources};

use crate::db::DbBridge;
use crate::skills_repo::SkillRow;
use crate::state::AppState;

/// 三份清单都从库与技能目录里现取 —— 唯一不「现取」的是 MCP 的连接
/// （那是内核 `mcp_client` 的事，这里只给已经查好的配置行）。
pub struct DbToolSources {
    /// 持 `Arc<AppState>` 而不是把 db / 配置拆开存：专家可见性走的是
    /// `api_experts::list_for_tools(&AppState, uid)` —— **全项目唯一的口径**。
    /// 为了转发给它就得有一份 AppState；在这里另拼一个 `ExpertRegistry`
    /// 等于给可见性规则开第二个出口（`list_for_tools` 的注释明说那会变成
    /// 绕过可见性的后门）。AppState 内部全是 Arc，克隆是廉价的。
    app: Arc<AppState>,
    /// SKILL 正文目录。**由调用方按 `api_extensions::skill_dir` 算好传进来**，
    /// 这里不重算：那是全项目唯一的目录口径，重算一份就会有两个真相
    /// （测试夹具也靠传自己的隔离目录，不读环境变量）。
    skill_root: PathBuf,
}

impl DbToolSources {
    pub fn new(app: Arc<AppState>, skill_root: PathBuf) -> Self {
        Self { app, skill_root }
    }

    /// 便捷构造：`&AppState` + 技能目录 → 可共享的端口句柄。
    /// 生产路径的三个调用点与集成测试都拿 `&AppState`，这一层把
    /// 「clone 一份 AppState 再包 Arc」收在一处。
    pub fn shared(state: &AppState, skill_root: PathBuf) -> Arc<dyn ToolSources> {
        Arc::new(Self::new(Arc::new(state.clone()), skill_root))
    }

    /// 存储不可用时的错误。**只在端口内部用**：既有调用点都在更早的地方
    /// 已经 `state.db()?` 过了，这里只是把同一条 ApiError 的消息原样带出去。
    fn db(&self) -> Result<&Arc<DbBridge>, ToolSourceError> {
        self.app.db().map_err(|e| ToolSourceError(e.to_string()))
    }
}

impl ToolSources for DbToolSources {
    fn experts_for_tools(&self, uid: UserId) -> Result<Vec<ExpertToolRow>, ToolSourceError> {
        let list = crate::api_experts::list_for_tools(&self.app, uid).map_err(ToolSourceError)?;
        Ok(list
            .into_iter()
            .map(|e| ExpertToolRow {
                id: e.id().as_str().to_string(),
                display_name: e.display_name().to_string(),
                description: e.description().to_string(),
                // 与 `resolve_persona` 同一种口径：空人格就是没有，不留空串。
                instructions: Some(e.instructions().to_string()).filter(|s| !s.trim().is_empty()),
            })
            .collect())
    }

    fn skills(&self, uid: UserId) -> Result<Vec<SkillToolRow>, ToolSourceError> {
        let db = self.db()?;
        let rows = crate::skills_repo::list_blocking(db, uid)
            .map_err(|e| ToolSourceError(e.to_string()))?;
        Ok(rows
            .iter()
            .map(|row| {
                // 正文路径由 `api_extensions::skill_body_path` 一处算出来 ——
                // 这里自己拼一份，就会出现「界面显示已启用、模型读不到正文」。
                // 算不出路径（DB 里名字不合法）就退化成「磁盘上没正文」，
                // 与「文件被删了」同一种形状：内核的 `NotMounted` 会照常
                // 报出「库里有行但磁盘上没有正文」，不另造一套说法。
                let body = match crate::api_extensions::skill_body_path(&self.skill_root, &row.name)
                {
                    Ok(p) => crate::api_extensions::read_skill_body(&p),
                    Err(_) => String::new(),
                };
                skill_tool_row(row, &body)
            })
            .collect())
    }

    fn mcp_servers(&self, uid: UserId) -> Result<Vec<McpServerRow>, ToolSourceError> {
        let db = self.db()?;
        crate::mcp_repo::list_blocking(db, uid).map_err(|e| ToolSourceError(e.to_string()))
    }
}

/// 壳侧的行（`skills_repo::SkillRow` + 已读好的正文）→ 内核的行。
///
/// 收在一处是因为 `api_extensions::list_skills` 也要这份映射（它用
/// `tools::skill_visibility` 报「模型看不看得见」）—— 两处各搬一遍字段，
/// 迟早有一边漏掉新字段，而那种漏在界面上看不出来。
pub fn skill_tool_row(row: &SkillRow, body: &str) -> SkillToolRow {
    SkillToolRow {
        name: row.name.clone(),
        description: row.description.clone(),
        enabled: row.enabled,
        // `skills.tool_allowlist_json` 2026-10-09 已删（Q102 / ISSUE-008）—— 内核那一行的
        // 同名字段留着是给 **MCP** 用的（`mcp_servers` 那一列还在），技能这一侧一律空。
        tool_allowlist: Vec::new(),
        body: body.to_string(),
    }
}
