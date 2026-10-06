//! 备份的三条 HTTP 接线：导出、校验，以及一条**诚实**的「不能在线还原」。
//!
//! 三条约束贯穿本文件：
//!
//! 1. **三条都只允许 admin**（`RequireAdmin`）。备份的源是整个数据根，
//!    那里有**所有**用户的会话全文；校验会读全量文件并回报目录级信息；
//!    还原那条虽不写盘，却会回显服务端绝对路径与一条可直接执行的命令。
//!    三者都是「实例级」操作，与 `/api/admin/config` 同类，不能只凭登录放行。
//! 2. `name` 只是备份根目录下的**相对名**，不是路径。浏览器无权决定服务端
//!    往哪儿写：绝对路径、盘符、`..` 一律 400，拼完之后还要过
//!    `pathsafe::is_within`（全项目唯一的包含判定）才作数。
//! 3. 校验在这里实现：`quill-backup` 只有导出与还原，没有 verify。判据就是
//!    「清单里记的摘要」与「文件当前内容重算的摘要」逐条对比，外加数据库快照
//!    摘要重算。**不许**另写一套「大致校验」—— 那正是备份功能最容易骗人的地方。
//! 4. 还原**不在进程内做**：服务正持有 SQLite 文件，边服务边覆盖它会写坏正在
//!    用的库。所以这条路由说清原因并给出停服后要跑的命令 —— 既不是 501
//!    （能力在 CLI 里是有的），也不是假成功。

use std::path::{Path, PathBuf};

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde_json::{json, Value};

use quill_agent::AgentError;
use quill_backup::manifest::unsafe_reason;
use quill_backup::{
    create_backup, sha256_file, BackupError, BackupSource, Manifest, MANIFEST_NAME,
};

use crate::api_experts::only_keys;
use crate::auth::RequireAdmin;
use crate::body::JsonBody;
use crate::error::ApiError;
use crate::state::AppState;

/// 备份根：`<实例根>/backups`，**在数据根之外**。
///
/// 实例根 = 数据根的上级（数据根就在实例根下，父目录为空时用 `.`）。
///
/// 为什么不能放在数据根里面：数据根就是被复制的源，备份根落在里面的话，
/// `create_backup` 会把「正在写的那个目录」一起复制进去，路径越滚越长，
/// 实测直接递归到 `File name too long` 并把存储线程的栈打爆（导出返 500）。
/// 所以这里是数据根的**兄弟**，不是它的子目录。
const BACKUP_SUBDIR: &str = "backups";

/// `quill-backup` 写出来的布局（那边是私有常量，没导出）。
/// 改那边就要同步改这里 —— 拼错了 verify 会把「文件缺失」报成校验不过。
const DB_SUBDIR: &str = "db";
const DATA_SUBDIR: &str = "data";
const DB_FILE: &str = "quill.db";

/// 校验时最多列出几处不符。全列出来能把一份坏备份的规模说清，
/// 但几十万行会撑爆响应与界面，所以截断并写明总数。
const MAX_REPORTED: usize = 5;

/// 库文件与备份根都从 `db_path` 推出来，不新增配置项：
/// 新配置项就意味着「界面能改的旋钮」，而这里没有第二处需要它。
fn data_root(state: &AppState) -> PathBuf {
    parent_or_dot(&state.config.db_path)
}

fn backup_root(state: &AppState) -> PathBuf {
    instance_root(&data_root(state)).join(BACKUP_SUBDIR)
}

/// 数据根的上级。`Path::parent()` 对 `data` 返回空串（不是 `None`），
/// 所以空串与 `None` 都要落到 `.` —— 否则相对路径布局下会退回到「备份根在数据根里」，
/// 也就是上面那条递归。
fn instance_root(data_root: &Path) -> PathBuf {
    parent_or_dot(data_root)
}

fn parent_or_dot(p: &Path) -> PathBuf {
    match p.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir.to_path_buf(),
        _ => PathBuf::from("."),
    }
}

struct Target {
    /// 原样回给客户端的相对名（响应里不带绝对路径以外的解释成本）。
    name: String,
    dir: PathBuf,
}

/// 把请求里的 `name` 解析成备份根下的一个目录。
///
/// 拒绝顺序有意如此：先判「这个字符串本身安不安全」，再判「拼出来的路径在不在
/// 根目录内」。前者给出可读的错因，后者兜住前者没想到的拼法。
fn resolve(state: &AppState, body: &Value, route: &str) -> Result<Target, ApiError> {
    only_keys(body, &["name"], route)?;
    let raw = body.get("name").and_then(Value::as_str).ok_or_else(|| {
        ApiError::bad_request(format!(
            "{route} 缺少字段 name（字符串）。\
             下一步：给一个备份根目录下的相对名，例如 `2026-10-06-01`。"
        ))
    })?;
    let name = raw.trim();

    // 复用 quill-backup 自己的 unsafe_reason：它已经是解析清单时的安全红线，
    // 「谁能指定一个文件名」不该有第二套口径。
    if let Some(why) = unsafe_reason(name) {
        return Err(ApiError::bad_request(format!(
            "备份名 {name:?} 不合法：{why}。\
             下一步：只给备份根目录下的相对名（例如 `2026-10-06-01`）；\
             绝对路径、盘符开头、含 `..` 一律不接受 —— 服务端不接受浏览器指定落盘位置。"
        )));
    }

    let root = backup_root(state);
    let dir = root.join(name);
    if !crate::pathsafe::is_within(&root, &dir) {
        return Err(ApiError::bad_request(format!(
            "备份名 {name:?} 解析后落在备份根目录 {} 之外，已拒绝。",
            root.display()
        )));
    }
    Ok(Target {
        name: name.to_string(),
        dir,
    })
}

/// `POST /api/backup/export` → 201。
///
/// **只允许 admin**（`RequireAdmin`，不是 `AuthUser`）：备份的源是整个数据根，
/// 而数据根里有**所有**用户的会话与消息 —— 普通登录用户一旦能导出，就等于
/// 拿到了别人的会话全文。这跟 `/api/admin/config` 是同一类「实例级」操作。
///
/// 目标非空、源不可用这两类在**进存储线程之前**就判掉：`create_backup`
/// 也会拒绝非空目标，但它给出的只是一句话，我们要给的是能照着做的
/// 409（带目录名）与 503（带真实原因）。
pub async fn export(
    State(state): State<AppState>,
    _admin: RequireAdmin,
    JsonBody(body): JsonBody,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let t = resolve(&state, &body, "POST /api/backup/export")?;

    let root = data_root(&state);
    if !state.config.db_path.is_file() {
        return Err(ApiError::service_unavailable(format!(
            "数据库文件 {} 不存在，备份源不可用。\
             下一步：用 QUILL_DB_PATH 指向一个真实存在的库文件后重启服务。",
            state.config.db_path.display()
        )));
    }
    if !root.is_dir() {
        return Err(ApiError::service_unavailable(format!(
            "用户数据目录 {} 不存在或不是目录，备份源不可用。\
             下一步：先执行 `quill doctor` 走一次初始化把该目录建出来，再重试导出。",
            root.display()
        )));
    }
    if t.dir.exists() && !t.dir.is_dir() {
        return Err(ApiError::bad_request(format!(
            "备份名 {} 指向一个已存在的文件（{}），不是目录。\
             下一步：换一个 name，或先把这个文件移走。",
            t.name,
            t.dir.display()
        )));
    }
    if t.dir.is_dir() && !dir_is_empty(&t.dir)? {
        return Err(ApiError::conflict(
            format!(
                "备份目标目录 {} 已存在且非空，拒绝写入 —— 覆盖它会毁掉上一份好备份。",
                t.dir.display()
            ),
            "换一个没用过的 name；或先把旧目录移走（`mv <目录> <目录>.old`，移走而不是删）。",
        ));
    }

    let db = state.db()?;
    let db_path = state.config.db_path.clone();
    let dest = t.dir.clone();
    // 池只在存储线程里拿得到（DbBridge 没有对外的池访问口），
    // 所以整段备份都跑在它自己的运行时上。
    let report = db
        .call(move |pool, _rt| {
            Box::pin(async move {
                let src = BackupSource::new(pool, db_path, root).map_err(backup_err)?;
                create_backup(&src, &dest).await.map_err(backup_err)
            })
        })
        .map_err(|e| call_failed("导出备份", e))?;

    Ok((
        StatusCode::CREATED,
        Json(json!({
            "name": t.name,
            "dest": report.dest.display().to_string(),
            "db_sha256": report.db_sha256,
            "files": report.manifest.files.len(),
            "total_bytes": report.manifest.total_bytes,
            "excluded": report.excluded
                .iter()
                .map(|e| json!({ "rel": e.rel, "reason": e.reason }))
                .collect::<Vec<Value>>(),
        })),
    ))
}

/// `POST /api/backup/verify` → 200（通过）或 422（逐条点名不符的文件）。
///
/// 同样**只允许 admin**：校验要逐个重算全量文件的 sha256，读的是整份备份内容，
/// 而且会把「哪些文件不在了」这条目录级信息回给调用方。
///
/// 失败**不是** `ok: false` 的 200：一份校验不过的备份不能被任何客户端
/// 读成「校验过了」。所以走标准错误信封，`detail` 里带路径与两个摘要。
///
/// 用 422 而不是 409：409 在本项目里表示「换个名字就能成」（目录已存在），
/// 校验失败换成名字不会有任何改变。
pub async fn verify(
    State(state): State<AppState>,
    _admin: RequireAdmin,
    JsonBody(body): JsonBody,
) -> Result<Json<Value>, ApiError> {
    let t = resolve(&state, &body, "POST /api/backup/verify")?;
    if !t.dir.is_dir() {
        return Err(ApiError::entity_not_found(format!(
            "备份根目录下没有名为 {} 的备份目录（{}）。\
             下一步：先 POST /api/backup/export 导出一份，再校验它。",
            t.name,
            t.dir.display()
        )));
    }
    let manifest_path = t.dir.join(MANIFEST_NAME);
    let text = std::fs::read_to_string(&manifest_path).map_err(|_| {
        ApiError::entity_not_found(format!(
            "目录 {} 里没有 {MANIFEST_NAME}，这不是一份完整备份。\
             下一步：换一份由 export 生成的备份来校验。",
            t.dir.display()
        ))
    })?;
    let manifest = Manifest::parse(&text).map_err(|e| {
        ApiError::unprocessable(
            format!("备份 {} 的清单无法解析：{e}", t.name),
            "这份备份的 manifest.json 已损坏或不是 quill 生成的格式，不能用于还原；\
             请改用另一份备份，或对该备份重新执行一次 export。",
        )
    })?;

    let mut bad: Vec<(String, String, String)> = Vec::new();

    let db_in_backup = t.dir.join(DB_SUBDIR).join(DB_FILE);
    let db_sha256 = match sha256_file(&db_in_backup) {
        Ok(d) => {
            if d != manifest.db_sha256 {
                bad.push((
                    format!("{DB_SUBDIR}/{DB_FILE}"),
                    manifest.db_sha256.clone(),
                    d.clone(),
                ));
            }
            d
        }
        Err(e) => {
            bad.push((
                format!("{DB_SUBDIR}/{DB_FILE}"),
                manifest.db_sha256.clone(),
                format!("读不出来（{e}）"),
            ));
            manifest.db_sha256.clone()
        }
    };

    let data_in_backup = t.dir.join(DATA_SUBDIR);
    for f in &manifest.files {
        let p = data_in_backup.join(&f.rel);
        // 清单解析已经过 unsafe_reason；这里再过一次包含判定，
        // 因为这里是把一个**外部文件**变成磁盘路径的那一步。
        if !crate::pathsafe::is_within(&data_in_backup, &p) {
            bad.push((
                f.rel.clone(),
                f.sha256.clone(),
                "路径不安全（解析后会落到备份数据目录之外）".to_string(),
            ));
            continue;
        }
        match sha256_file(&p) {
            Ok(actual) if actual == f.sha256 => {}
            Ok(actual) => bad.push((f.rel.clone(), f.sha256.clone(), actual)),
            Err(e) => bad.push((f.rel.clone(), f.sha256.clone(), format!("读不出来（{e}）"))),
        }
    }

    if !bad.is_empty() {
        return Err(ApiError::unprocessable(
            format!(
                "备份 {} 校验未通过：{n} 处内容与清单不符（已逐个重算 sha256 与清单对比）。\n{lines}",
                t.name,
                n = bad.len(),
                lines = bad
                    .iter()
                    .take(MAX_REPORTED)
                    .map(|(rel, expect, actual)| {
                        format!("  · {rel}：清单记 {expect}，实际 {actual}\n")
                    })
                    .collect::<String>(),
            ),
            "这份备份不可用于还原 —— 切勿跳过校验强行恢复。\
             请改用另一份备份；确认损坏原因可执行 `sha256sum '<备份目录>/<路径>'` 复核。",
        ));
    }

    Ok(Json(json!({
        "name": t.name,
        "ok": true,
        "files": manifest.files.len(),
        "total_bytes": manifest.total_bytes,
        "db_sha256": db_sha256,
    })))
}

/// `POST /api/backup/restore` → 200，但**没有**还原。
///
/// 同样**只允许 admin**：这条虽不写盘，但它会把**服务端绝对路径**与
/// 库文件位置回显给调用方，并把一条可直接照抄的 `quill restore` 命令交出去。
/// 普通用户拿到这两样就等于拿到了实例布局，不该给。
///
/// 服务进程通过 `DbBridge` 一直开着 SQLite 文件；在线覆盖它等于在服务还在
/// 读写的时候换掉底下的库。所以这里说清原因，并给出停服后要跑的**真实**命令
/// （`quill restore <目录> --db <库> --root <数据根> --yes`，标志名取自
/// `quill-cli/src/cmd_backup.rs`；注意 CLI 的 restore 把 `--root` 直接当数据根用）。
pub async fn restore(
    State(state): State<AppState>,
    _admin: RequireAdmin,
    JsonBody(body): JsonBody,
) -> Result<Json<Value>, ApiError> {
    let t = resolve(&state, &body, "POST /api/backup/restore")?;
    if !t.dir.is_dir() {
        return Err(ApiError::entity_not_found(format!(
            "备份根目录下没有名为 {} 的备份目录（{}），没有可还原的东西。\
             下一步：先 POST /api/backup/export 导出一份，再停服还原。",
            t.name,
            t.dir.display()
        )));
    }

    let db_path = state.config.db_path.display().to_string();
    let root = data_root(&state).display().to_string();
    let command = format!(
        "quill restore {} --db {db_path} --root {root} --yes",
        t.dir.display()
    );

    Ok(Json(json!({
        "restored": false,
        "name": t.name,
        "backup_dir": t.dir.display().to_string(),
        "db_path": db_path,
        "data_root": root,
        "reason": format!(
            "运行中的服务正持有数据库文件（{db_path}）。在线还原会在服务还在读写它的时候\
             覆盖文件，可能写坏正在服务的库，所以这一步不在进程内做 —— 本次请求**没有**还原任何东西。"
        ),
        "command": command,
        "next_step": "先停掉 quill-server，再在终端执行上面那条命令；\
                      还原完成后再启动服务。恢复是破坏性的：现有数据会被替换。",
    })))
}

fn dir_is_empty(dir: &Path) -> Result<bool, ApiError> {
    let mut rd = std::fs::read_dir(dir)
        .map_err(|e| ApiError::internal(format!("读取备份目标目录 {} 失败：{e}", dir.display())))?;
    Ok(rd.next().is_none())
}

/// `BackupError` 只能以文本跨过 `db.call` 的边界（那里只收 `AgentError`），
/// 所以把 crate 自己写的修复命令一并带上，别在服务端把它丢掉。
///
/// 目标非空与源缺失在进存储线程之前就判掉了，因此落到这里的都属于
/// 「真出了别的问题」，报 500 而不是伪装成 409/503。
fn backup_err(e: BackupError) -> AgentError {
    AgentError::Storage {
        detail: e.render_with_fix(),
    }
}

fn call_failed(op: &str, e: AgentError) -> ApiError {
    eprintln!("[api] {op} 失败（错误码 {}）：{e}", e.code());
    ApiError::internal(format!("{op}失败：{e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 名字本身不安全的一律 400。这条挡的是「浏览器指定服务端写哪儿」。
    #[test]
    fn hostile_names_are_refused_with_the_reason() {
        for bad in [
            "../escape",
            "..\\escape",
            "C:\\Windows\\x",
            "/etc/passwd",
            "",
            "   ",
        ] {
            let why = unsafe_reason(bad.trim()).unwrap_or_else(|| panic!("{bad:?} 应被判不安全"));
            assert!(!why.is_empty(), "{bad:?} 必须给出可读原因");
        }
    }

    /// 反过来，正常的相对名（含子目录）不能被误拒。
    #[test]
    fn ordinary_relative_names_are_accepted() {
        for good in ["2026-10-06-01", "daily/2026-10-06", "a"] {
            assert!(unsafe_reason(good).is_none(), "{good:?} 是合法备份名");
        }
    }
}
