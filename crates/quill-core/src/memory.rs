//! 记忆（memory）—— 照 goose 的 **memory MCP 扩展**移植。
//!
//! **移植出处**：`vendor/goose/crates/goose-mcp/src/memory/mod.rs`（851 行，
//! goose v1.53.0）。goose 把「记忆」做成一个**内置的 MCP 扩展**：`goose-mcp/src/lib.rs`
//! 的 `BUILTIN_EXTENSIONS` 里有 `memory` 一项（`pub use memory::MemoryServer`），
//! 对外是四个工具 —— `remember_memory` / `retrieve_memories` /
//! `remove_memory_category` / `remove_specific_memory` —— 背后是**按分类落盘的一批
//! `.txt`**。同一份服务器还有一条 stdio 入口 `goose mcp memory`
//! （`goose-mcp/src/mcp_server_runner.rs` 的 `serve`，由 `goose-cli` 的 `Command::Mcp`
//! 分派），quill 照这条走：`quill mcp memory`。
//!
//! **这是「记忆」在 quill 里的第一处实现**（queue Q019：此前为零）。此前队列把它记为
//! 「卡在：goose 没有 memory 原语」—— **那条是错的**：`grep -rl memory vendor/goose/crates/goose/src/`
//! 只扫了 `goose` 这一 crate，漏了 `goose-mcp`。本模块按 `goose-mcp` 的原样移植。
//!
//! ## 抄了什么、没抄什么
//!
//! **抄的**（语义逐字对齐上游）：
//! - 存储形状：一个分类 = 一个 `.txt`；一次 `remember` 追加「可选 `# 标签行` + 正文 + 空行」；
//!   一次 `retrieve` 按**空行**切条目，首行以 `#` 开头就是标签、否则归入 `untagged`。
//! - 两种作用域：**本地**（项目内）与**全局**（用户配置目录）；`retrieve_all` / 清空可跨分类。
//! - 分类名是**单个文件名分量**：空、`*`、`.`、`..`、含 `/`、`\`、`:`，以及 Windows 保留名
//!   （`CON` / `PRN` / `AUX` / `NUL` / `CLOCK$` / `COM1..9` / `LPT1..9`，含上标 ¹²³）一律拒绝。
//!   这是**安全边界**：分类名来自模型，不校验就等于把任意路径交给它写。
//! - 全局记忆在启动时被**拼进服务器 instructions**（`new()` 里的 `Global Memories:` 段）。
//!
//! **没抄 / 与上游不同**（记进 `docs/UPSTREAM-DIVERGENCES.md`）：
//! - 本地目录 `.goose/memory` → **`.quill/memory`**（quill 是自己的产品，用 `.goose` 是错的）。
//! - 全局目录用 `default_global_memory_dir()` 现算（`%APPDATA%\quill\memory` /
//!   `$XDG_CONFIG_HOME/quill/memory` / `$HOME/.config/quill/memory`），**没有引入 goose 用的
//!   `etcetera`** —— 平台目录只有这两支，为它加一个依赖不划算；口径写在这里以便核对。
//! - goose 把内置扩展跑在**进程内**（`tokio::io::duplex`，见 `goose/src/agents/extension_manager/builtin.rs`）；
//!   quill 走它自己的 stdio 那条（`goose mcp memory` 的等价物），因为 quill 的 `mcp_client`
//!   只铺了 stdio。功能等价，形状不同。
//!
//! ## 这一层的边界
//!
//! 本模块只负责**记忆的读写与它的 MCP 工具面**。把它**接进某轮对话**（让模型自动
//! 调得到）要靠壳侧把 `quill mcp memory` 登记成一台 MCP 服务器 —— 与 Q017/Q020/Q022
//! 一样，内核逻辑先落地、接线另记。

use std::collections::HashMap;
use std::fs;
use std::io::{self, Read, Write};
use std::path::PathBuf;

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{
    CallToolResult, ContentBlock, ErrorCode, ErrorData, Implementation, InitializeResult,
    MetaObject, ServerCapabilities, ServerConfig,
};
use rmcp::service::RequestContext;
use rmcp::{tool, tool_handler, tool_router, RoleServer, ServerHandler, ServiceExt};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// 请求元数据里携带「当前工作目录」的键。goose 同名字段（上游 `mod.rs:21`）。
///
/// 本地记忆落在**工作目录**下，而工作目录随调用方而变，所以它走 MCP 的 `_meta`
/// 传进来，而不是服务器自己猜 `current_dir()`。
const WORKING_DIR_HEADER: &str = "agent-working-dir";

/// 本地记忆的目录名（相对工作目录）。上游是 `.goose/memory`，quill 用自己的 `.quill`。
const LOCAL_MEMORY_DIR: &str = ".quill/memory";

/// Windows 保留设备名。分类名会变成一个文件名，落到这些名字上在 Windows 上会失败
/// （或更糟，被解释成设备）。上游 `is_reserved_windows_category`（`mod.rs:23-40`）逐字搬。
fn is_reserved_windows_category(category: &str) -> bool {
    let basename = category
        .split('.')
        .next()
        .unwrap_or(category)
        .trim_end_matches([' ', '.']);
    let uppercase = basename.to_ascii_uppercase();

    matches!(uppercase.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CLOCK$")
        || ["COM", "LPT"].iter().any(|prefix| {
            uppercase.strip_prefix(prefix).is_some_and(|suffix| {
                matches!(
                    suffix,
                    "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                )
            })
        })
}

/// 从 MCP 请求的 `_meta` 里取工作目录。取不到就是 `None`（调用方退回 `current_dir`）。
fn extract_working_dir_from_meta(meta: &MetaObject) -> Option<PathBuf> {
    meta.0
        .get(WORKING_DIR_HEADER)
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
}

/// 把 IO 错误转成 MCP 错误：**参数不合法**与**内部错误**要分开报，
/// 否则调用方分不清「我传错了」和「服务器坏了」。上游 `memory_error`（`mod.rs:50-57`）。
fn memory_error(error: io::Error) -> ErrorData {
    let code = if error.kind() == io::ErrorKind::InvalidInput {
        ErrorCode::INVALID_PARAMS
    } else {
        ErrorCode::INTERNAL_ERROR
    };
    ErrorData::new(code, error.to_string(), None)
}

/// 全局记忆目录：用户级配置目录下的 `quill/memory`。
///
/// 现算而不引 `etcetera`（goose 用的那个）：平台目录只有两支，
/// Windows 看 `%APPDATA%`，其余看 `$XDG_CONFIG_HOME` / `$HOME/.config`。
/// 都取不到时退回 `./.quill-global/memory`（**不 panic** —— 取不到家目录不该让整个
/// 服务器起不来；退回一个相对路径，至少读写在同处）。
pub fn default_global_memory_dir() -> PathBuf {
    #[cfg(windows)]
    {
        if let Ok(appdata) = std::env::var("APPDATA") {
            if !appdata.is_empty() {
                return PathBuf::from(appdata).join("quill").join("memory");
            }
        }
    }
    #[cfg(not(windows))]
    {
        if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
            if !xdg.is_empty() {
                return PathBuf::from(xdg).join("quill").join("memory");
            }
        }
        if let Ok(home) = std::env::var("HOME") {
            if !home.is_empty() {
                return PathBuf::from(home)
                    .join(".config")
                    .join("quill")
                    .join("memory");
            }
        }
    }
    PathBuf::from(".quill-global").join("memory")
}

// --------------------------------------------------------------- 工具参数（照上游）

/// `remember_memory` 的参数。字段与上游 `RememberMemoryParams`（`mod.rs:60-71`）一一对应。
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct RememberMemoryParams {
    /// The category to store the memory in
    pub category: String,
    /// The data to remember
    pub data: String,
    /// Optional tags for the memory
    #[serde(default)]
    pub tags: Vec<String>,
    /// Whether to store globally or locally
    pub is_global: bool,
}

/// `retrieve_memories` 的参数（上游 `RetrieveMemoriesParams`，`mod.rs:74-80`）。
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct RetrieveMemoriesParams {
    /// The category to retrieve memories from (use "*" for all)
    pub category: String,
    /// Whether to retrieve from global or local storage
    pub is_global: bool,
}

/// `remove_memory_category` 的参数（上游 `RemoveMemoryCategoryParams`，`mod.rs:83-89`）。
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct RemoveMemoryCategoryParams {
    /// The category to remove (use "*" for all)
    pub category: String,
    /// Whether to remove from global or local storage
    pub is_global: bool,
}

/// `remove_specific_memory` 的参数（上游 `RemoveSpecificMemoryParams`，`mod.rs:92-100`）。
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct RemoveSpecificMemoryParams {
    /// The category containing the memory
    pub category: String,
    /// The content of the memory to remove
    pub memory_content: String,
    /// Whether to remove from global or local storage
    pub is_global: bool,
}

// ------------------------------------------------------------------- 服务器

/// 记忆 MCP 服务器。上游 `MemoryServer`（`mod.rs:102-108`）。
///
/// `global_memory_dir` 是**注入**的（上游也这样），所以测试不用碰真实家目录。
#[derive(Clone)]
pub struct MemoryServer {
    tool_router: ToolRouter<Self>,
    instructions: String,
    global_memory_dir: PathBuf,
}

impl Default for MemoryServer {
    fn default() -> Self {
        Self::new()
    }
}

/// 服务器对模型说的那段话（instructions）。上游 `mod.rs:119-131`，路径改成 quill 的。
fn base_instructions() -> String {
    format!(
        "This extension stores and retrieves categorized information with tagging support.\n\
         \n\
         Storage:\n\
         - Local: {LOCAL_MEMORY_DIR}/ (project-specific)\n\
         - Global: <user config dir>/quill/memory/ (user-wide)\n\
         \n\
         Save proactively when users share preferences, project configurations, workflow patterns,\n\
         or recurring commands. Always confirm with the user before saving. Suggest relevant\n\
         categories and tags, and clarify storage scope (local vs global).\n\
         \n\
         Use category \"*\" with retrieve_memories or remove_memory_category to access all entries."
    )
}

#[tool_router(router = tool_router)]
impl MemoryServer {
    /// 建一个服务器：全局目录走 `default_global_memory_dir()`，并把**已有的全局记忆
    /// 拼进 instructions**（上游 `mod.rs:118-173` 同一套：启动时就把全局记忆交给模型）。
    pub fn new() -> Self {
        Self::with_global_dir(default_global_memory_dir())
    }

    /// 指定全局目录建服务器（测试用；生产走 `new()`）。
    pub fn with_global_dir(global_memory_dir: PathBuf) -> Self {
        let mut server = Self {
            tool_router: Self::tool_router(),
            instructions: base_instructions(),
            global_memory_dir,
        };
        server.instructions = server.instructions_with_global_memories();
        server
    }

    /// 在基础 instructions 后面接一段「用户当前已存的全局记忆」。读不到就只留基础段
    /// （**不报错** —— 一段说明文字不值得让服务器起不来）。
    fn instructions_with_global_memories(&self) -> String {
        let mut out = base_instructions();
        out.push_str(
            "\n\n**Here are the user's currently saved memories:**\n\
             Please keep this information in mind when answering future questions.\n\
             Do not bring up memories unless relevant.\n\
             Note: if the user has not saved any memories, this section will be empty.\n\
             Note: if the user removes a memory that was previously loaded into the system, \
             please remove it from the system instructions.",
        );
        if let Ok(global_memories) = self.retrieve_all(true, None) {
            if !global_memories.is_empty() {
                out.push_str("\n\nGlobal Memories:\n");
                for (category, memories) in global_memories {
                    out.push_str(&format!("\nCategory: {}\n", category));
                    for memory in memories {
                        out.push_str(&format!("- {}\n", memory));
                    }
                }
            }
        }
        out
    }

    pub fn get_instructions(&self) -> &str {
        &self.instructions
    }

    /// 某个分类的记忆文件路径。**分类名先过白名单**（单个文件名分量），否则报
    /// `InvalidInput`。上游 `get_memory_file`（`mod.rs:184-215`）。
    pub fn get_memory_file(
        &self,
        category: &str,
        is_global: bool,
        working_dir: Option<&PathBuf>,
    ) -> io::Result<PathBuf> {
        if category.is_empty()
            || category == "*"
            || category == "."
            || category == ".."
            || category.contains('/')
            || category.contains('\\')
            || category.contains(':')
            || is_reserved_windows_category(category)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "memory category must be a single filename component",
            ));
        }

        Ok(self
            .base_dir(is_global, working_dir)
            .join(format!("{}.txt", category)))
    }

    /// 记忆的根目录：全局目录，或工作目录下的本地目录。上游 `mod.rs:205-213 / 222-230`。
    fn base_dir(&self, is_global: bool, working_dir: Option<&PathBuf>) -> PathBuf {
        if is_global {
            self.global_memory_dir.clone()
        } else {
            let local_base = working_dir
                .cloned()
                .or_else(|| std::env::current_dir().ok())
                .unwrap_or_else(|| PathBuf::from("."));
            local_base.join(LOCAL_MEMORY_DIR)
        }
    }

    /// 读回**所有**分类，返回「分类 → 条目正文列表」。目录不存在就是空。
    /// 文件名不是 `.txt`、或分类名不合法（历史遗留）的**跳过**，不报错。
    /// 上游 `retrieve_all`（`mod.rs:217-258`）。
    pub fn retrieve_all(
        &self,
        is_global: bool,
        working_dir: Option<&PathBuf>,
    ) -> io::Result<HashMap<String, Vec<String>>> {
        let base_dir = self.base_dir(is_global, working_dir);
        let mut memories = HashMap::new();
        if base_dir.exists() {
            for entry in fs::read_dir(&base_dir)? {
                let entry = entry?;
                if entry.file_type()?.is_file() {
                    let file_name = entry.file_name();
                    let Some(category) = file_name
                        .to_str()
                        .and_then(|name| name.strip_suffix(".txt"))
                    else {
                        continue;
                    };
                    if self
                        .get_memory_file(category, is_global, working_dir)
                        .is_err()
                    {
                        continue;
                    }
                    let category_memories = self.retrieve(category, is_global, working_dir)?;
                    memories.insert(
                        category.to_string(),
                        category_memories.into_values().flatten().collect(),
                    );
                }
            }
        }
        Ok(memories)
    }

    /// 追加一条记忆。**追加**（`append`）而不是覆盖：同一个分类里的历史条目要留住。
    /// 有标签就写一行 `# tag1 tag2`，然后写正文，然后一个空行（空行是条目分隔符）。
    /// 上游 `remember`（`mod.rs:260-285`）。
    pub fn remember(
        &self,
        category: &str,
        data: &str,
        tags: &[&str],
        is_global: bool,
        working_dir: Option<&PathBuf>,
    ) -> io::Result<()> {
        let memory_file_path = self.get_memory_file(category, is_global, working_dir)?;

        if let Some(parent) = memory_file_path.parent() {
            fs::create_dir_all(parent)?;
        }

        let mut file = fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(&memory_file_path)?;
        if !tags.is_empty() {
            writeln!(file, "# {}", tags.join(" "))?;
        }
        writeln!(file, "{}\n", data)?;

        Ok(())
    }

    /// 读一个分类，返回「标签（无标签是 `untagged`）→ 该标签下的行」。文件不存在就是空。
    /// 上游 `retrieve`（`mod.rs:287-325`）。
    pub fn retrieve(
        &self,
        category: &str,
        is_global: bool,
        working_dir: Option<&PathBuf>,
    ) -> io::Result<HashMap<String, Vec<String>>> {
        let memory_file_path = self.get_memory_file(category, is_global, working_dir)?;
        if !memory_file_path.exists() {
            return Ok(HashMap::new());
        }

        let mut file = fs::File::open(memory_file_path)?;
        let mut content = String::new();
        file.read_to_string(&mut content)?;

        let mut memories = HashMap::new();
        for entry in content.split("\n\n") {
            let mut lines = entry.lines();
            if let Some(first_line) = lines.next() {
                if let Some(stripped) = first_line.strip_prefix('#') {
                    let tags = stripped
                        .split_whitespace()
                        .map(String::from)
                        .collect::<Vec<_>>();
                    memories.insert(tags.join(" "), lines.map(String::from).collect());
                } else {
                    let entry_data: Vec<String> = std::iter::once(first_line.to_string())
                        .chain(lines.map(String::from))
                        .collect();
                    memories
                        .entry("untagged".to_string())
                        .or_insert_with(Vec::new)
                        .extend(entry_data);
                }
            }
        }

        Ok(memories)
    }

    /// 从一个分类里删掉**内容包含** `memory_content` 的条目，其余原样重写。
    /// 上游 `remove_specific_memory_internal`（`mod.rs:327-353`）。
    ///
    /// **口径是「包含」不是「相等」**（上游如此）：给一段更短的子串，会连带删掉
    /// 所有含它的条目。保留上游行为，不擅自收紧 —— 改了就是另一种语义。
    pub fn remove_specific_memory_internal(
        &self,
        category: &str,
        memory_content: &str,
        is_global: bool,
        working_dir: Option<&PathBuf>,
    ) -> io::Result<()> {
        let memory_file_path = self.get_memory_file(category, is_global, working_dir)?;
        if !memory_file_path.exists() {
            return Ok(());
        }

        let mut file = fs::File::open(&memory_file_path)?;
        let mut content = String::new();
        file.read_to_string(&mut content)?;

        let memories: Vec<&str> = content.split("\n\n").collect();
        let new_content: Vec<String> = memories
            .into_iter()
            .filter(|entry| !entry.contains(memory_content))
            .map(|s| s.to_string())
            .collect();

        fs::write(memory_file_path, new_content.join("\n\n"))?;

        Ok(())
    }

    /// 清空一个分类（删文件）。上游 `clear_memory`（`mod.rs:355-367`）。
    pub fn clear_memory(
        &self,
        category: &str,
        is_global: bool,
        working_dir: Option<&PathBuf>,
    ) -> io::Result<()> {
        let memory_file_path = self.get_memory_file(category, is_global, working_dir)?;
        if memory_file_path.exists() {
            fs::remove_file(memory_file_path)?;
        }

        Ok(())
    }

    /// 清空**所有**分类（删整个记忆目录）。上游
    /// `clear_all_global_or_local_memories`（`mod.rs:369-387`）。
    pub fn clear_all(&self, is_global: bool, working_dir: Option<&PathBuf>) -> io::Result<()> {
        let base_dir = self.base_dir(is_global, working_dir);
        if base_dir.exists() {
            fs::remove_dir_all(&base_dir)?;
        }
        Ok(())
    }

    // ------------------------------------------------------------- MCP 工具

    /// Stores a memory with optional tags in a specified category
    #[tool(
        name = "remember_memory",
        description = "Stores a memory with optional tags in a specified category"
    )]
    pub async fn remember_memory(
        &self,
        params: Parameters<RememberMemoryParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        let working_dir = extract_working_dir_from_meta(&context.meta);

        if params.data.is_empty() {
            return Err(ErrorData::new(
                ErrorCode::INVALID_PARAMS,
                "Data must not be empty when remembering a memory".to_string(),
                None,
            ));
        }

        let tags: Vec<&str> = params.tags.iter().map(|s| s.as_str()).collect();
        self.remember(
            &params.category,
            &params.data,
            &tags,
            params.is_global,
            working_dir.as_ref(),
        )
        .map_err(memory_error)?;

        Ok(CallToolResult::success(vec![ContentBlock::text(format!(
            "Stored memory in category: {}",
            params.category
        ))]))
    }

    /// Retrieves all memories from a specified category
    #[tool(
        name = "retrieve_memories",
        description = "Retrieves all memories from a specified category"
    )]
    pub async fn retrieve_memories(
        &self,
        params: Parameters<RetrieveMemoriesParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        let working_dir = extract_working_dir_from_meta(&context.meta);

        let memories = if params.category == "*" {
            self.retrieve_all(params.is_global, working_dir.as_ref())
        } else {
            self.retrieve(&params.category, params.is_global, working_dir.as_ref())
        }
        .map_err(memory_error)?;

        Ok(CallToolResult::success(vec![ContentBlock::text(format!(
            "Retrieved memories: {:?}",
            memories
        ))]))
    }

    /// Removes all memories within a specified category
    #[tool(
        name = "remove_memory_category",
        description = "Removes all memories within a specified category"
    )]
    pub async fn remove_memory_category(
        &self,
        params: Parameters<RemoveMemoryCategoryParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        let working_dir = extract_working_dir_from_meta(&context.meta);

        let message = if params.category == "*" {
            self.clear_all(params.is_global, working_dir.as_ref())
                .map_err(memory_error)?;
            format!(
                "Cleared all memory {} categories",
                if params.is_global { "global" } else { "local" }
            )
        } else {
            self.clear_memory(&params.category, params.is_global, working_dir.as_ref())
                .map_err(memory_error)?;
            format!("Cleared memories in category: {}", params.category)
        };

        Ok(CallToolResult::success(vec![ContentBlock::text(message)]))
    }

    /// Removes a specific memory within a specified category
    #[tool(
        name = "remove_specific_memory",
        description = "Removes a specific memory within a specified category"
    )]
    pub async fn remove_specific_memory(
        &self,
        params: Parameters<RemoveSpecificMemoryParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        let working_dir = extract_working_dir_from_meta(&context.meta);

        self.remove_specific_memory_internal(
            &params.category,
            &params.memory_content,
            params.is_global,
            working_dir.as_ref(),
        )
        .map_err(memory_error)?;

        Ok(CallToolResult::success(vec![ContentBlock::text(format!(
            "Removed specific memory from category: {}",
            params.category
        ))]))
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for MemoryServer {
    fn get_info(&self) -> ServerConfig {
        InitializeResult::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(
                "quill-memory",
                env!("CARGO_PKG_VERSION"),
            ))
            .with_instructions(self.instructions.clone())
    }
}

/// 把记忆服务器跑在 stdio 上（`quill mcp memory` 的实现）。
///
/// 与上游 `goose-mcp/src/mcp_server_runner.rs` 的 `serve`（`ServiceExt::serve(stdio())`
/// → `waiting()`）同一套调用序列。
///
/// **stdout 只能出协议内容**：调用方（`quill-cli` 的 main）绝不能在这里往 stdout
/// 打别的字，否则客户端解析就断。失败信息一律走 stderr。
pub async fn serve_stdio() -> Result<(), String> {
    let server = MemoryServer::new();
    let service = server
        .serve(rmcp::transport::stdio())
        .await
        .map_err(|e| format!("记忆服务器起不来：{e}"))?;
    service
        .waiting()
        .await
        .map_err(|e| format!("记忆服务器异常退出：{e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 每个用例一个独立工作目录（不引 `tempfile`：全仓的临时目录约定就是
    /// `std::env::temp_dir()` + 进程/线程 id，见 `quill-store/src/lib.rs:584`）。
    fn workdir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "quill-memory-{}-{}-{:?}",
            tag,
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    fn server(tag: &str) -> (MemoryServer, PathBuf, PathBuf) {
        let work = workdir(tag);
        let global = work.join("global-config");
        (MemoryServer::with_global_dir(global.clone()), work, global)
    }

    #[test]
    fn remember_then_retrieve_is_a_real_read_write_loop() {
        let (s, work, _) = server("rw");
        let wd = Some(&work);

        s.remember("notes", "用户偏好中文回复", &["pref"], false, wd)
            .expect("写");

        let got = s.retrieve("notes", false, wd).expect("读");
        let flat: Vec<&String> = got.values().flatten().collect();
        assert!(
            flat.iter().any(|l| l.contains("用户偏好中文回复")),
            "读回来的内容里必须有刚写进去的那条，实际 {got:?}"
        );
        // 真落盘了：本地文件在工作目录下（不是只在内存里）。
        let file = work.join(LOCAL_MEMORY_DIR).join("notes.txt");
        assert!(file.is_file(), "本地记忆应落在 {file:?}");
        let raw = fs::read_to_string(&file).unwrap();
        assert!(
            raw.contains("# pref"),
            "标签要写成 `# pref` 行，实际 {raw:?}"
        );
    }

    #[test]
    fn remember_creates_only_the_local_dir_not_the_global_one() {
        let (s, work, global) = server("lazy");
        assert!(!work.join(LOCAL_MEMORY_DIR).exists());
        assert!(!global.exists());

        s.remember("c", "d", &[], false, Some(&work)).unwrap();
        assert!(work.join(LOCAL_MEMORY_DIR).exists());
        assert!(!global.exists(), "写本地不该顺手把全局目录也建出来");

        s.remember("c", "d", &[], true, None).unwrap();
        assert!(global.exists(), "写全局应建出全局目录");
    }

    #[test]
    fn tagged_and_untagged_entries_are_separated() {
        let (s, work, _) = server("tags");
        let wd = Some(&work);
        s.remember("c", "带标签的", &["a", "b"], false, wd).unwrap();
        s.remember("c", "没标签的", &[], false, wd).unwrap();

        let got = s.retrieve("c", false, wd).unwrap();
        assert!(
            got.contains_key("a b"),
            "两个标签应合成键 `a b`，实际 {got:?}"
        );
        assert!(got.contains_key("untagged"), "无标签条目归 `untagged`");
    }

    #[test]
    fn clear_removes_the_category_and_retrieve_comes_back_empty() {
        let (s, work, _) = server("clear");
        let wd = Some(&work);
        s.remember("c", "x", &[], false, wd).unwrap();
        assert!(!s.retrieve("c", false, wd).unwrap().is_empty());

        s.clear_memory("c", false, wd).unwrap();
        assert!(s.retrieve("c", false, wd).unwrap().is_empty());
    }

    #[test]
    fn remove_specific_memory_keeps_the_other_entries() {
        let (s, work, _) = server("rm-one");
        let wd = Some(&work);
        s.remember("c", "keep_this", &[], false, wd).unwrap();
        s.remember("c", "remove_this", &[], false, wd).unwrap();

        s.remove_specific_memory_internal("c", "remove_this", false, wd)
            .unwrap();

        let flat: Vec<String> = s
            .retrieve("c", false, wd)
            .unwrap()
            .into_values()
            .flatten()
            .collect();
        assert!(flat.iter().any(|l| l.contains("keep_this")));
        assert!(!flat.iter().any(|l| l.contains("remove_this")));
    }

    #[test]
    fn retrieve_all_spans_categories() {
        let (s, work, _) = server("all");
        let wd = Some(&work);
        s.remember("one", "first", &[], false, wd).unwrap();
        s.remember("two", "second", &[], false, wd).unwrap();

        let all = s.retrieve_all(false, wd).unwrap();
        assert_eq!(all.len(), 2, "两个分类都要出现，实际 {all:?}");
    }

    #[test]
    fn clear_all_removes_every_category() {
        let (s, work, _) = server("clear-all");
        let wd = Some(&work);
        s.remember("one", "first", &[], false, wd).unwrap();
        s.remember("two", "second", &[], false, wd).unwrap();

        s.clear_all(false, wd).unwrap();
        assert!(s.retrieve_all(false, wd).unwrap().is_empty());
    }

    #[test]
    fn escape_capable_categories_are_rejected_before_touching_the_filesystem() {
        let (s, work, _) = server("escape");
        // 造一个「工作目录之外」的文件，确保它一个字节都没被碰。
        let outside = work.parent().unwrap().join(format!(
            "quill-memory-outside-{}-{:?}.txt",
            std::process::id(),
            std::thread::current().id()
        ));
        fs::write(&outside, "secret").unwrap();

        for category in [
            "",
            "*",
            ".",
            "..",
            "../../../outside",
            "/tmp/outside",
            r"..\..\outside",
            "C:outside",
            r"C:\outside",
            "NUL",
            "con",
            "AUX.log",
            "COM1",
            "lpt9",
        ] {
            assert_eq!(
                s.remember(category, "malicious", &[], false, Some(&work))
                    .unwrap_err()
                    .kind(),
                io::ErrorKind::InvalidInput,
                "分类 {category:?} 必须被拒"
            );
            assert_eq!(
                s.retrieve(category, false, Some(&work)).unwrap_err().kind(),
                io::ErrorKind::InvalidInput
            );
            assert_eq!(
                s.remove_specific_memory_internal(category, "secret", false, Some(&work))
                    .unwrap_err()
                    .kind(),
                io::ErrorKind::InvalidInput
            );
            assert_eq!(
                s.clear_memory(category, false, Some(&work))
                    .unwrap_err()
                    .kind(),
                io::ErrorKind::InvalidInput
            );
        }

        assert_eq!(fs::read_to_string(&outside).unwrap(), "secret");
        assert!(
            !work.join(LOCAL_MEMORY_DIR).exists(),
            "被拒的分类不该留下任何目录"
        );
        let _ = fs::remove_file(&outside);
    }

    #[test]
    fn safe_filename_characters_are_allowed() {
        let (s, work, _) = server("safe-name");
        s.remember("project notes_2026", "safe", &[], false, Some(&work))
            .unwrap();
        assert!(work
            .join(LOCAL_MEMORY_DIR)
            .join("project notes_2026.txt")
            .is_file());
    }

    #[test]
    fn retrieve_all_skips_invalid_legacy_categories() {
        let (s, work, _) = server("legacy");
        let dir = work.join(LOCAL_MEMORY_DIR);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("valid.txt"), "kept").unwrap();
        // 历史遗留：文件名里带 `:`（分类规则后来才收紧）。
        fs::write(dir.join("work:api.txt"), "legacy").unwrap();

        let all = s.retrieve_all(false, Some(&work)).unwrap();
        assert_eq!(all.len(), 1, "非法分类名要跳过，实际 {all:?}");
        assert!(all["valid"].iter().any(|e| e == "kept"));
    }

    #[test]
    fn global_memories_are_folded_into_the_instructions() {
        let work = workdir("instr");
        let global = work.join("global-config");
        let s = MemoryServer::with_global_dir(global.clone());
        s.remember("prefs", "用户叫阿甲", &[], true, None).unwrap();

        // 新起一台（模拟下一次启动）—— 全局记忆应出现在 instructions 里。
        let fresh = MemoryServer::with_global_dir(global);
        assert!(
            fresh.get_instructions().contains("用户叫阿甲"),
            "全局记忆要被拼进 instructions，实际：{}",
            fresh.get_instructions()
        );
        assert!(fresh.get_instructions().contains("Global Memories:"));
    }

    #[test]
    fn memory_error_maps_invalid_input_apart_from_internal() {
        let invalid = memory_error(io::Error::new(io::ErrorKind::InvalidInput, "bad category"));
        assert_eq!(invalid.code, ErrorCode::INVALID_PARAMS);

        let filesystem = memory_error(io::Error::new(io::ErrorKind::PermissionDenied, "denied"));
        assert_eq!(filesystem.code, ErrorCode::INTERNAL_ERROR);
    }

    #[test]
    fn the_tool_router_exposes_the_four_goose_tools() {
        // 工具面照上游：四个名字一个不多一个不少（名字是模型看到的东西，不能漂）。
        let s = MemoryServer::with_global_dir(workdir("tools").join("g"));
        let names: Vec<String> = s
            .tool_router
            .list_all()
            .into_iter()
            .map(|t| t.name.to_string())
            .collect();
        for want in [
            "remember_memory",
            "retrieve_memories",
            "remove_memory_category",
            "remove_specific_memory",
        ] {
            assert!(
                names.iter().any(|n| n == want),
                "缺工具 {want}，实际 {names:?}"
            );
        }
        assert_eq!(names.len(), 4, "只应有这四个工具，实际 {names:?}");
    }
}
