//! 升级的三条 HTTP 接线：查版本、升级前备份、升级历史（queue Q038–Q040）。
//!
//! 三条都是 **admin-only**（`RequireAdmin`），口径与备份三条一致（见
//! `api_backup.rs` 的文件头）：升级要替换程序与库，`prepare` 会写出整份数据根的
//! 备份，`history` 会回显服务端绝对路径。这些都是「实例级」操作，只凭登录
//! 放行等于把所有人的数据交给任何一个普通账号。
//!
//! ## Q038 `GET /api/upgrade/check`：有没有新版只能来自**真来源**
//!
//! 仓库里没有任何内置更新源，所以这个问题不能凭空回答：
//!
//! - 没配 `QUILL_UPGRADE_MANIFEST_URL` → `has_update` / `latest_version` 都是
//!   `null`，`note` 说明「未配置更新源，无法判断」；
//! - 配了 → 真去 GET 那个 URL，按 [`VERSION_RULE`] 真比较版本；
//! - 配了但取不到（连不上 / 非 2xx / 不是 JSON / 缺 `version`）→ 同样是
//!   `null` + `source_error` 说明原因。
//!
//! **任何失败路径都不会出现 `has_update: false`** —— 「取不到」不是「已是最新」，
//! 把前者写成后者就是编数字。`false` 只在一个时候出现：真的取到了清单，而且
//! 按规则比出「清单里的版本不高于当前版本」。
//!
//! 当前版本取自本 crate 的 `CARGO_PKG_VERSION`，与 `GET /api/version`
//! （`routes.rs::version`）逐字同源：运行中的二进制就是这个 crate 编出来的，
//! 版本号必须跟着二进制走，不是 workspace 里别的 crate 的版本。
//!
//! ## Q039 `POST /api/upgrade/prepare`：真落备份
//!
//! 调 `quill_upgrade::take_pre_upgrade_backup`（`PreUpgradeGuard` 的既有入口，
//! 它此前零调用方），把备份真写进备份根（与 `api_backup` 同一个
//! `实例根/backups`），响应里给 `backup_dir` 与 `BackupReport` 的字段。
//! 备份失败**绝不返回成功**：错误文案原样带上 `UpgradeError` 里那段中文
//! （「升级前备份失败，升级已被阻断……不要跳过备份直接升级」）。
//!
//! 该接口**只做备份，不做升级**：进程内的升级（换二进制、跑迁移）不在 HTTP 里
//! 干，所以响应里 `upgraded: false` 并给出停服后的回滚命令。备份本身是一份
//! 普通 quill 备份，所以 `POST /api/backup/verify` 能校验它、`quill restore`
//! 能拿它回滚。
//!
//! ## Q040 `GET /api/upgrade/history`：历史来自真落盘的 JSONL
//!
//! 每次 `prepare` 成功后在数据根下追加一行 `upgrade-history.jsonl`
//! （时间 / 版本 / 备份目录 / 报告摘要）。列出时逐行读回：
//! 文件不存在 → 空列表 + note 说清「还没有升级记录」；文件存在但读不出来或
//! 有一行解析不了 → 5xx 如实报错，**不静默跳过**（跳过才会造出一份「少一条记录」
//! 的假历史）。没有记录与读不出来，在状态码上就能分开（200 空 vs 5xx）。

use std::cmp::Ordering;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde_json::{json, Value};

use quill_agent::AgentError;
use quill_backup::{BackupError, BackupSource};
use quill_upgrade::{take_pre_upgrade_backup, PreUpgradeGuard, UpgradeError};

use crate::api_backup;
use crate::auth::RequireAdmin;
use crate::error::ApiError;
use crate::state::AppState;

/// 当前版本的唯一口径：运行中的二进制所属 crate 的版本。
/// 与 `routes.rs` 里 `GET /api/version` 用的是同一个 `CARGO_PKG_VERSION`。
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

/// 更新源配置项：一个返回 `{"version": ..., "url"?: ..., "notes"?: ..., "sha256"?: ...}` 的
/// JSON 清单 URL。
///
/// `sha256` 是 `POST /api/upgrade/apply` 的**硬要求**（没有它就拒绝下载），
/// `GET /api/upgrade/check` 不读它。
///
/// 每次请求现读环境变量（与 `skillhub::http::host` 读 `QUILL_SKILLHUB_HOST` 同构）：
/// 换源不用重启，测试也不必为了注入来源去改 `Config` 的结构。
pub const MANIFEST_URL_ENV: &str = "QUILL_UPGRADE_MANIFEST_URL";

/// 版本比较规则，原样写进响应：客户端读到的 `has_update` 是怎么比较出来的，
/// 不该靠猜。
pub const VERSION_RULE: &str =
    "语义化版本逐段比较：允许一个前导 v/V，按 . 分段，每段必须是非空的纯十进制数字；\
     缺少的段按 0 补（0.1 == 0.1.0），多出的段照常比较（1.0.0.1 > 1.0.0）；\
     任一段不是纯数字（如 0.2.0-rc1、latest）或数值溢出 u64，即视为不可比较 —— \
     此时 has_update 是 null（不知道），不是 false（已是最新）。";

/// 升级历史文件（JSONL，一行一条）。放数据根下：它和用户数据一样是实例状态，
/// 会跟着普通备份一起被带走。
const HISTORY_FILE: &str = "upgrade-history.jsonl";

/// 更新清单的响应体上限。清单只有几个字段，256 KiB 是「绝无可能不够」的量；
/// 没有上限的话，一个超大响应会先被读进内存再谈解析（见 `skillhub::http` 的同类防护）。
const MAX_MANIFEST_BYTES: usize = 256 * 1024;

const FETCH_TIMEOUT_SECS: u64 = 10;

const USER_AGENT: &str = "quill-upgrade-check/1.0";

// ---------------------------------------------------------------------------
// Q038 · GET /api/upgrade/check
// ---------------------------------------------------------------------------

/// `GET /api/upgrade/check` → 200，永远回答三件事之一：有新版 / 没有新版 / 不知道。
///
/// 「不知道」也用 200：这条请求**成功地**完成了（去问了来源、或确认没有来源），
/// 答案本身是「不知道」而已。把「不知道」塞进错误码只会让客户端把原因丢掉；
/// 这里用 `has_update: null` + `source_error` + `note` 把原因说全。
pub async fn check(_admin: RequireAdmin) -> Json<Value> {
    let current = APP_VERSION;
    let url = manifest_url();

    let mut latest_version: Option<String> = None;
    let mut release_url: Option<String> = None;
    let mut release_notes: Option<String> = None;
    let mut source_error: Option<String> = None;
    let mut has_update: Option<bool> = None;
    let note: String;

    match &url {
        None => {
            note = format!(
                "未配置更新源：环境变量 {MANIFEST_URL_ENV} 未设置（或为空串），\
                 服务端没有任何可以问到「最新版本」的地方 —— \
                 所以 has_update / latest_version 都是 null，这是「不知道」，\
                 不是「已是最新」。下一步：若要启用检查，把 {MANIFEST_URL_ENV} 设成\
                 一个 JSON 清单的 URL（形状：{{\"version\":\"0.2.0\",\"url\":\"…\",\"notes\":\"…\",\"sha256\":\"…\"}}；\
                 `sha256` 只有 `POST /api/upgrade/apply` 用得到），\
                 再刷新本接口；没有清单源时，请以发布说明为准，不要把 null 当绿灯。"
            );
        }
        Some(u) => match fetch_manifest(u).await {
            Err(e) => {
                source_error = Some(e);
                note = format!(
                    "已配置更新源 {u}，但这次取不到清单（原因见 source_error）。\
                     因此无法判断有没有新版：has_update / latest_version 都是 null。\
                     取不到**不等于**已是最新，也不等于有更新。\
                     下一步：确认该 URL 可达（例如 `curl -sS '{u}'`）、\
                     返回 2xx 且是 {{\"version\": …}} 形状，修好后重试。"
                );
            }
            Ok(m) => {
                latest_version = Some(m.version.clone());
                release_url = m.url;
                release_notes = m.notes;
                match compare_versions(current, &m.version) {
                    Some(Ordering::Less) => {
                        has_update = Some(true);
                        note = format!(
                            "更新源 {u} 报的最新版本 {} 高于当前版本 {current} —— 有新版。\
                             下一步：先 POST /api/upgrade/prepare 落一份升级前备份，再按发布说明升级。",
                            m.version
                        );
                    }
                    Some(Ordering::Equal) => {
                        has_update = Some(false);
                        note = format!(
                            "更新源 {u} 报的最新版本与当前版本相同（{}）—— 没有新版。",
                            m.version
                        );
                    }
                    Some(Ordering::Greater) => {
                        has_update = Some(false);
                        note = format!(
                            "更新源 {u} 报的最新版本 {} **低于**当前版本 {current}：\
                             当前版本比更新源还新（预发布 / 本地构建时常见），没有可升级的新版。\
                             这不是「取不到」—— 清单确实读到了，比较也确实做了。",
                            m.version
                        );
                    }
                    None => {
                        note = format!(
                            "更新源 {u} 报的 version「{}」按比较规则不可比较，无法判断有没有新版 \
                             —— has_update 是 null，不是 false。规则：{VERSION_RULE}",
                            m.version
                        );
                    }
                }
            }
        },
    }

    Json(json!({
        "current_version": current,
        "has_update": has_update,
        "latest_version": latest_version,
        "release_url": release_url,
        "notes": release_notes,
        "source": if url.is_some() { "manifest_url" } else { "none" },
        "source_url": url,
        "source_error": source_error,
        "version_rule": VERSION_RULE,
        "note": note,
    }))
}

fn manifest_url() -> Option<String> {
    std::env::var(MANIFEST_URL_ENV)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

#[derive(Debug, Clone)]
struct RemoteManifest {
    version: String,
    url: Option<String>,
    notes: Option<String>,
    /// 产物文件的 sha256（64 位十六进制，大小写不敏感）。
    ///
    /// **`POST /api/upgrade/apply` 只认带它的清单**：没有它就拒绝下载 ——
    /// 「不校验就落盘」等于把任意文件写进数据目录，那比不升级坏得多。
    /// `GET /api/upgrade/check` 不需要它（只是查版本），所以它是可选的。
    sha256: Option<String>,
}

/// 真去取清单。失败原因整句返回（写进响应的 `source_error`），
/// 调用方负责把它和「不知道」这个结论一起说清楚。
async fn fetch_manifest(url: &str) -> Result<RemoteManifest, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(FETCH_TIMEOUT_SECS))
        .redirect(reqwest::redirect::Policy::limited(5))
        .user_agent(USER_AGENT)
        .build()
        .map_err(|e| format!("构造 HTTP 客户端失败：{e}"))?;
    let resp = client
        .get(url)
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|e| format!("请求更新清单失败：{e}"))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(format!(
            "更新清单返回 HTTP {}（只有 2xx 才算取到；非 2xx 的响应体一概不解析）",
            status.as_u16()
        ));
    }
    let bytes = read_capped(resp, MAX_MANIFEST_BYTES, "更新清单").await?;
    let v: Value = serde_json::from_slice(&bytes)
        .map_err(|e| format!("更新清单不是合法 JSON（读了 {} 字节）：{e}", bytes.len()))?;
    let version = v
        .get("version")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            "更新清单缺少非空的字符串字段 version（形状应为 {\"version\": \"0.2.0\", \"url\"?: \"…\", \"notes\"?: \"…\", \"sha256\"?: \"…\"}；`sha256` 是 `POST /api/upgrade/apply` 才需要的，`GET /api/upgrade/check` 不需要它）"
                .to_string()
        })?
        .to_string();
    Ok(RemoteManifest {
        version,
        url: v.get("url").and_then(Value::as_str).map(str::to_string),
        notes: v.get("notes").and_then(Value::as_str).map(str::to_string),
        sha256: v
            .get("sha256")
            .and_then(Value::as_str)
            .map(|s| s.trim().to_ascii_lowercase())
            .filter(|s| !s.is_empty()),
    })
}

/// 边读边限量：越界那一刻就停，不把整个响应先吃进内存。
async fn read_capped(resp: reqwest::Response, cap: usize, what: &str) -> Result<Vec<u8>, String> {
    let mut resp = resp;
    let mut buf: Vec<u8> = Vec::new();
    while let Some(chunk) = resp
        .chunk()
        .await
        .map_err(|e| format!("读取{what}失败：{e}"))?
    {
        if buf.len() + chunk.len() > cap {
            return Err(format!(
                "{what}超过 {cap} 字节上限，已停止读取 —— 这么大的响应按配置错误处理"
            ));
        }
        buf.extend_from_slice(&chunk);
    }
    Ok(buf)
}

/// 按 [`VERSION_RULE`] 比较两个版本。
///
/// 返回 `None` 就是「不可比较」：调用方必须据此给出 `has_update: null`，
/// **不许**退化成「相等」或「不更新」。
fn compare_versions(current: &str, latest: &str) -> Option<Ordering> {
    let a = parse_version(current)?;
    let b = parse_version(latest)?;
    for i in 0..a.len().max(b.len()) {
        let x = a.get(i).copied().unwrap_or(0);
        let y = b.get(i).copied().unwrap_or(0);
        match x.cmp(&y) {
            Ordering::Equal => continue,
            other => return Some(other),
        }
    }
    Some(Ordering::Equal)
}

fn parse_version(text: &str) -> Option<Vec<u64>> {
    let t = text.trim();
    let t = t.strip_prefix(['v', 'V']).unwrap_or(t);
    if t.is_empty() {
        return None;
    }
    let mut parts = Vec::new();
    for seg in t.split('.') {
        if seg.is_empty() || !seg.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        parts.push(seg.parse::<u64>().ok()?);
    }
    Some(parts)
}

// ---------------------------------------------------------------------------
// Q063 · POST /api/upgrade/apply —— 下载 + 校验 + 暂存，替换动作交给人
// ---------------------------------------------------------------------------

/// 产物文件的下载上限（256 MiB）。与清单同一个道理：没有上限就会先被读进内存。
const MAX_ARTIFACT_BYTES: usize = 256 * 1024 * 1024;

/// 下载产物的超时。比取清单（10s）宽松得多 —— 产物可能几十 MB。
const DOWNLOAD_TIMEOUT_SECS: u64 = 300;

/// 暂存根目录名。放在数据根下，与 `backups/` 同级。
const STAGING_DIR: &str = "upgrade-staging";

fn is_sha256_hex(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// 只留安全字符。**版本号与文件名都来自外部清单**，不能直接当路径用。
fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .take(64)
        .collect()
}

/// 产物文件名：取 URL 路径的最后一段（**去掉查询串**）；取不到或只剩点就用固定名。
fn file_name_from(url: &str) -> String {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    let last = path.trim_end_matches('/').rsplit('/').next().unwrap_or("");
    let cleaned = sanitize(last);
    if cleaned.is_empty() || cleaned.chars().all(|c| c == '.') {
        "quill-artifact".to_string()
    } else {
        cleaned
    }
}

/// `POST /api/upgrade/apply` —— 把新版本**下载并校验后暂存**，真正的替换动作交给人
/// （连同现成的命令）。
///
/// ## 为什么不是「进程内把自己换掉」
///
/// 正在跑的二进制在 Windows 上**被锁着**（换不掉），在 Unix 上换掉之后内存里跑的仍是旧
/// 代码 —— 所以在线升级的真实形态只能是：**下载 → 校验 → 暂存 → 停服 → 换文件 → 起服**。
/// 这个端点做前三步，后三步交出去（与 `POST /api/backup/restore` 同一个口径：
/// 进程内做不到的就把真命令交出来，而不是假装做到了）。响应里 `upgraded` **恒为 false**。
///
/// ## 三条硬规矩（每条都当场给出可照着做的下一步）
///
/// 1. **清单必须带 `sha256`**，否则拒绝下载（409）—— 不校验就落盘等于把任意文件写进
///    数据目录，比不升级坏得多。
/// 2. **清单版本必须真的比当前新**（按 [`VERSION_RULE`] 比较）；相等 / 更低 / 不可比较
///    都拒绝（409）—— 不可比较时**不许**当成「有新版本」。
/// 3. **校验不过就什么都不留**（422，点名两个摘要）：暂存目录只在**校验通过之后**才建。
pub async fn apply(
    State(state): State<AppState>,
    _admin: RequireAdmin,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let current = APP_VERSION;
    let Some(source) = manifest_url() else {
        return Err(ApiError::conflict(
            format!("没有配更新源（{MANIFEST_URL_ENV}），没有可下载的东西。当前版本 {current}。"),
            "先配好更新源，再用 GET /api/upgrade/check 确认能取到清单",
        ));
    };
    let m = fetch_manifest(&source).await.map_err(|e| {
        ApiError::upstream_unavailable(
            format!("取更新清单失败：{e}（来源 {source}）"),
            "确认更新源可达、返回 2xx 且是 {\"version\":…,\"url\":…,\"sha256\":…} 形状后重试",
        )
    })?;

    let Some(artifact_url) = m.url.clone().filter(|u| !u.trim().is_empty()) else {
        return Err(ApiError::conflict(
            format!(
                "更新清单里没有 `url`：只知道最新版本是 {}，没有可下载的产物。",
                m.version
            ),
            "让更新源在清单里补上 url（指向产物文件）",
        ));
    };
    let Some(expected) = m.sha256.clone() else {
        return Err(ApiError::conflict(
            format!(
                "更新清单里没有 `sha256`，**拒绝下载**：不校验摘要就落盘，等于把任意文件\
                 写进数据目录 —— 那比不升级坏得多。（清单里的版本是 {}。）",
                m.version
            ),
            "让更新源在清单里补上产物的 sha256（64 位十六进制）",
        ));
    };
    if !is_sha256_hex(&expected) {
        return Err(ApiError::conflict(
            format!("更新清单里的 sha256 {expected:?} 不是 64 位十六进制，没法用它校验。"),
            "按 64 位十六进制写 sha256（大小写都接受）",
        ));
    }
    // 注意方向：`compare_versions(current, latest)` 比的是**当前**与清单的顺序，
    // 所以「有新版」= 当前 **小于** 清单版本（`Less`）。写反了会把「没有新版」
    // 当成「有新版」—— 本轮就是被新测试当场抓住的。
    match compare_versions(current, &m.version) {
        Some(Ordering::Less) => {}
        Some(Ordering::Equal) | Some(Ordering::Greater) => {
            return Err(ApiError::conflict(
                format!(
                    "清单版本 {} 不高于当前版本 {current}（相等或更低），没有可升级的新版。",
                    m.version
                ),
                "不必升级；要强制重装请手工替换二进制",
            ))
        }
        None => {
            return Err(ApiError::conflict(
                format!(
                    "清单里的 version「{}」按比较规则不可比较，不敢据此下载。规则：{VERSION_RULE}",
                    m.version
                ),
                "让更新源给出可比较的版本号",
            ))
        }
    }

    // 下载：与取清单同一套防护（超时 / 重定向上限 / 边读边限量）。
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(DOWNLOAD_TIMEOUT_SECS))
        .redirect(reqwest::redirect::Policy::limited(5))
        .user_agent(USER_AGENT)
        .build()
        .map_err(|e| ApiError::internal(format!("构造下载客户端失败：{e}")))?;
    let resp = client.get(&artifact_url).send().await.map_err(|e| {
        ApiError::upstream_unavailable(
            format!("下载产物失败（{artifact_url}）：{e}"),
            "确认产物 URL 可达后重试",
        )
    })?;
    let status = resp.status();
    if !status.is_success() {
        return Err(ApiError::upstream_unavailable(
            format!("下载产物返回 HTTP {}（只有 2xx 才算拿到）", status.as_u16()),
            "确认产物 URL 返回 2xx",
        ));
    }
    let bytes = read_capped(resp, MAX_ARTIFACT_BYTES, "产物文件")
        .await
        .map_err(|e| ApiError::upstream_unavailable(e, "确认产物大小与更新源配置"))?;

    // **校验在落盘之前**：不过就什么都不留。
    let actual = quill_backup::sha256_bytes(&bytes);
    if actual != expected {
        return Err(ApiError::unprocessable(
            format!(
                "产物摘要对不上：清单说 {expected}，实测 {actual}（{} 字节，来源 {artifact_url}）。\
                 **一个字节都没落盘。**",
                bytes.len()
            ),
            "让更新源重新生成 sha256（或换一个产物源）再重试",
        ));
    }

    // 校验通过才建暂存目录。`create_dir` 是原子的：同秒重复提交只会有一个成功
    // （与 `reserve_backup_dir` 同款，避免两次提交共用一个目录名）。
    let staging_root = api_backup::data_root(&state).join(STAGING_DIR);
    let name = format!("{}-{}", sanitize(&m.version), now_unix());
    let dir = staging_root.join(&name);
    std::fs::create_dir_all(&staging_root).map_err(|e| {
        ApiError::internal(format!("建暂存根 {} 失败：{e}", staging_root.display()))
    })?;
    match std::fs::create_dir(&dir) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            return Err(ApiError::conflict(
                format!("暂存目录 {} 已存在（同一秒重复提交？）", dir.display()),
                "隔一秒重试",
            ))
        }
        Err(e) => {
            return Err(ApiError::internal(format!(
                "建暂存目录 {} 失败：{e}",
                dir.display()
            )))
        }
    }
    let staged = dir.join(file_name_from(&artifact_url));
    std::fs::write(&staged, &bytes)
        .map_err(|e| ApiError::internal(format!("写暂存文件 {} 失败：{e}", staged.display())))?;
    // 暂存目录里留一份「这是什么」的小清单：排障时不必去猜那个文件是哪来的。
    let meta = json!({
        "version": m.version,
        "sha256": actual,
        "source_url": artifact_url,
        "staged_at_unix": now_unix(),
        "bytes": bytes.len(),
        "notes": m.notes,
    });
    let _ = std::fs::write(
        dir.join("manifest.json"),
        serde_json::to_vec_pretty(&meta).unwrap_or_default(),
    );

    let exe = std::env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "<运行中的 quill-server 路径>".to_string());
    let steps = json!([
        "1. 停掉正在跑的服务（按你的启动方式：systemd `systemctl stop …` / 前台 Ctrl-C / 计划任务）",
        format!("2. 先备份旧的那份：mv {exe} {exe}.old"),
        format!("3. 换上新的：mv {} {exe}", staged.display()),
        "4. 重新启动服务，再 `curl /api/version` 确认版本号已变；有问题就把 .old 换回来",
    ]);

    Ok((
        StatusCode::OK,
        Json(json!({
            "upgraded": false,
            "current_version": current,
            "latest_version": m.version,
            "sha256": actual,
            "expected_sha256": expected,
            "bytes": bytes.len(),
            "staged_file": staged.display().to_string(),
            "staged_dir": dir.display().to_string(),
            "next_steps": steps,
            "note": "这个进程**没有**升级自己：正在跑的二进制在 Windows 上被锁着、在 Unix 上换了内存里也仍是旧代码。所以这里只做「下载 → 校验 → 暂存」，替换与重启按 next_steps 做。",
        })),
    ))
}

// ---------------------------------------------------------------------------
// Q039 · POST /api/upgrade/prepare
// ---------------------------------------------------------------------------

/// `POST /api/upgrade/prepare` → 201：落一份**可校验**的升级前备份，并记账。
///
/// 只做备份，不做升级（见文件头）。备份目标由服务端自己取名
/// （`pre-upgrade-<毫秒>`，碰撞加后缀）：浏览器无权指定落盘位置，
/// 与 `api_backup` 同一条红线。
pub async fn prepare(
    State(state): State<AppState>,
    _admin: RequireAdmin,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let db = state.db()?;
    let db_path = state.config.db_path.clone();
    let data_root = api_backup::data_root(&state);

    // 与 api_backup::export 同款前置判定：这两类是「环境坏了」，
    // 给 503 + 能照着做的下一步，而不是让 create_backup 从深处抛一句文言。
    if !db_path.is_file() {
        return Err(ApiError::service_unavailable(format!(
            "数据库文件 {} 不存在，升级前备份不可用，升级已被阻断。\
             下一步：用 QUILL_DB_PATH 指向一个真实存在的库文件后重启服务；\
             不要跳过备份直接升级 —— 迁移失败会留下一个无人能修的库。",
            db_path.display()
        )));
    }
    if !data_root.is_dir() {
        return Err(ApiError::service_unavailable(format!(
            "用户数据目录 {} 不存在或不是目录，升级前备份不可用，升级已被阻断。\
             下一步：先执行 `quill doctor` 走一次初始化把该目录建出来，再重试。",
            data_root.display()
        )));
    }

    // 备份根与 api_backup 同源（数据根的兄弟目录）：所以这份备份是**普通备份**，
    // POST /api/backup/verify 能校验、quill restore 能回滚。
    let backups_root = api_backup::backup_root(&state);
    let (name, dest) = reserve_backup_dir(&backups_root)?;

    // 备份源要进存储线程（`'static`），所以数据根与库路径各克隆一份进去；
    // 本函数后面还要用 `data_root` 算历史文件路径与回滚命令。
    let backup_data_root = data_root.clone();
    let outcome = db
        .call(move |pool, _rt| {
            Box::pin(async move {
                let src = match BackupSource::new(pool, db_path, backup_data_root) {
                    Ok(src) => src,
                    Err(source) => {
                        return Ok::<_, AgentError>(Err(UpgradeError::PreUpgradeBackupFailed {
                            dest,
                            source,
                        }));
                    }
                };
                Ok(take_pre_upgrade_backup(&src, dest, APP_VERSION).await)
            })
        })
        .map_err(|e| call_failed("升级前备份", e))?;
    let guard: PreUpgradeGuard = outcome.map_err(pre_upgrade_failure)?;

    let report = guard.report();
    let record = HistoryRecord {
        at_unix: now_unix(),
        app_version: APP_VERSION.to_string(),
        backup_name: name.clone(),
        backup_dir: guard.backup_dir().display().to_string(),
        db_sha256: report.db_sha256.clone(),
        files: report.manifest.files.len(),
        total_bytes: report.manifest.total_bytes,
    };
    // 备份已经落盘，才算记账。记账失败不改写「备份成功」这个事实，
    // 但必须如实说出来（见响应里的 history_appended / history_error）。
    let (history_appended, history_error) = match append_history(&data_root, &record) {
        Ok(()) => (true, None),
        Err(e) => (false, Some(e)),
    };

    let restore_command = format!(
        "quill restore {} --db {} --root {} --yes",
        guard.backup_dir().display(),
        state.config.db_path.display(),
        data_root.display()
    );

    Ok((
        StatusCode::CREATED,
        Json(json!({
            "prepared": true,
            // 升级本身不在进程内做：这条接口只保证「升级前备份已就绪」。
            "upgraded": false,
            "app_version": APP_VERSION,
            "backup_name": name,
            "backup_dir": guard.backup_dir().display().to_string(),
            "created_unix": report.manifest.created_unix,
            "db_bytes": report.manifest.db_bytes,
            "db_sha256": report.db_sha256,
            "files": report.manifest.files.len(),
            "total_bytes": report.manifest.total_bytes,
            "excluded": report
                .excluded
                .iter()
                .map(|e| json!({ "rel": e.rel, "reason": e.reason }))
                .collect::<Vec<Value>>(),
            "history_appended": history_appended,
            "history_error": history_error,
            "history_path": history_path(&data_root).display().to_string(),
            "next_step": format!(
                "升级前备份已完成（{name}）；升级本身不在这条接口里执行。\
                 下一步：停掉服务后按发布说明升级，失败时执行 `{restore_command}` 回到升级前；\
                 这份备份也可以先用 POST /api/backup/verify 校验。"
            ),
        })),
    ))
}

/// 服务端自己取的名字：`pre-upgrade-<毫秒>`（重名加 `-2`、`-3`……）。
///
/// 用 `create_dir` 的**原子性**占位，而不是「先 exists 再 join」：后者的 TOCTOU
/// 窗口在并发下是真实的 —— 两个同毫秒的请求会拿到同一个名字，一起往同一个
/// `.part` 文件里写（实测：第二个 VACUUM INTO 报「table schema_version already
/// exists」，备份内容不可信）。谁把目录建出来谁就拥有这个名字。
///
/// 预留出来的空目录 `create_backup` 是接受的（它只拒绝**非空**目录），
/// 所以这里建完直接交给它写。
fn reserve_backup_dir(backups_root: &Path) -> Result<(String, PathBuf), ApiError> {
    std::fs::create_dir_all(backups_root).map_err(|e| {
        ApiError::internal(format!(
            "创建备份根目录 {} 失败：{e}。升级前备份是硬性守卫，\
             这一步不成功升级不得继续。下一步：检查该路径的属主与磁盘空间\
             （`df -h`），修好后重试。",
            backups_root.display()
        ))
    })?;
    let base = format!("pre-upgrade-{}", now_millis());
    for n in 1..=100u32 {
        let candidate = if n == 1 {
            base.clone()
        } else {
            format!("{base}-{n}")
        };
        let dest = backups_root.join(&candidate);
        match std::fs::create_dir(&dest) {
            Ok(()) => return Ok((candidate, dest)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => {
                return Err(ApiError::internal(format!(
                    "创建备份目录 {} 失败：{e}。下一步：检查属主与磁盘空间后重试。",
                    dest.display()
                )))
            }
        }
    }
    Err(ApiError::internal(format!(
        "备份根 {} 下 pre-upgrade-* 连续 100 个名字都被占用，无法为这次升级预留目录。\
         下一步：先清理多年不用的同名目录（移走而不是删），或换一个备份根。",
        backups_root.display()
    )))
}

/// `UpgradeError` → HTTP 错误。`text` 用的是 guard 自己那段中文
/// （「升级前备份失败，升级已被阻断……不要跳过备份直接升级」），
/// 只在外面补上分类与状态码。
fn pre_upgrade_failure(e: UpgradeError) -> ApiError {
    let text = e.to_string();
    match &e {
        UpgradeError::PreUpgradeBackupFailed { source, .. } => match source {
            BackupError::DbMissing { .. } | BackupError::DataRootMissing { .. } => {
                ApiError::service_unavailable(text)
            }
            BackupError::DestNotEmpty { .. } => ApiError::conflict(
                text,
                "换一个没用过的备份名，或把那个目录移开（`mv <目录> <目录>.old`，移走而不是删）。\
                 升级前备份是硬性守卫：这一步不成功，升级不得继续。",
            ),
            _ => ApiError::internal(text),
        },
        // prepare 从不调 rollback（本接口不做升级），所以这条分支出现即是缺陷，
        // 按内部错误报，不编一个「回滚失败」的假故事。
        UpgradeError::RollbackFailed { .. } => ApiError::internal(text),
    }
}

fn call_failed(op: &str, e: AgentError) -> ApiError {
    eprintln!("[api] {op} 失败（错误码 {}）：{e}", e.code());
    ApiError::internal(format!("{op}失败：{e}"))
}

// ---------------------------------------------------------------------------
// Q040 · GET /api/upgrade/history
// ---------------------------------------------------------------------------

/// 一条升级记录。字段就是「时间 / 版本 / 备份目录 / 报告摘要」，
/// 全部来自 `prepare` 当场拿到的真实数据 —— 没有任何一处是补出来的。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct HistoryRecord {
    at_unix: u64,
    app_version: String,
    backup_name: String,
    backup_dir: String,
    db_sha256: String,
    files: usize,
    total_bytes: u64,
}

fn history_path(data_root: &Path) -> PathBuf {
    data_root.join(HISTORY_FILE)
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn now_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

/// 追加一行。用 `append` 打开（O_APPEND 下的一次 write 不会覆盖已有记录），
/// 写完 `sync_all`：历史是「真落盘」的，不能停在页缓存里。
fn append_history(data_root: &Path, rec: &HistoryRecord) -> Result<(), String> {
    use std::io::Write;

    let path = history_path(data_root);
    let mut line = serde_json::to_string(rec).map_err(|e| format!("序列化升级记录失败：{e}"))?;
    line.push('\n');
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| format!("打开升级历史 {} 失败：{e}", path.display()))?;
    f.write_all(line.as_bytes())
        .map_err(|e| format!("写入升级历史 {} 失败：{e}", path.display()))?;
    f.sync_all()
        .map_err(|e| format!("把升级历史 {} 刷到磁盘失败：{e}", path.display()))?;
    Ok(())
}

/// `GET /api/upgrade/history` → 200 列表；读不出来时 5xx，绝不返回「空列表」。
///
/// 「没有记录」与「读不出来」必须能分开：前者是 200 + `entries: []` + note，
/// 后者是 5xx + detail（文件在哪、哪一行、为什么）。把读失败吞成空列表，
/// 用户会以为「从来没升级过」，而真实情况是记录丢了或坏了。
pub async fn history(
    State(state): State<AppState>,
    _admin: RequireAdmin,
) -> Result<Json<Value>, ApiError> {
    let data_root = api_backup::data_root(&state);
    let path = history_path(&data_root);

    if !path.exists() {
        return Ok(Json(json!({
            "entries": [],
            "count": 0,
            "path": path.display().to_string(),
            "note": format!(
                "还没有升级记录：{} 不存在。它会在第一次 POST /api/upgrade/prepare \
                 成功落下备份后创建；这份空列表是「确实没有记录」，不是读失败。",
                path.display()
            ),
        })));
    }

    let text = std::fs::read_to_string(&path).map_err(|e| {
        ApiError::internal(format!(
            "升级历史读不出来：{} 存在，但读取失败（{e}）。\
             这**不是**「没有记录」。下一步：检查该路径是不是一个普通文件、\
             属主与权限是否对服务进程可读；修好之前不要把它当成空历史。",
            path.display()
        ))
    })?;

    let mut entries: Vec<HistoryRecord> = Vec::new();
    for (idx, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        let rec: HistoryRecord = serde_json::from_str(line).map_err(|e| {
            ApiError::internal(format!(
                "升级历史第 {} 行解析不了（{e}）：这一行可能被手工改过或写到一半。\
                 历史文件：{}。下一步：先把它复制一份留底，再逐行核对修复或删掉坏行；\
                 不许静默跳过 —— 跳过会造出一份「少一条记录」的假历史。",
                idx + 1,
                path.display()
            ))
        })?;
        entries.push(rec);
    }

    let count = entries.len();
    let note = if count == 0 {
        Some(format!(
            "升级历史文件 {} 存在，但里面还没有任何记录行 —— 空列表是真的没有记录。",
            path.display()
        ))
    } else {
        None
    };

    Ok(Json(json!({
        "entries": entries,
        "count": count,
        "path": path.display().to_string(),
        "note": note,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use quill_backup::{sha256_file, MANIFEST_NAME};

    /// 比较规则逐条钉死。规则变一句，这里就要改一处 —— 这正是响应里
    /// `version_rule` 那句人话所描述的东西。
    #[test]
    fn version_comparison_follows_the_documented_rule() {
        assert_eq!(compare_versions("0.1.0", "0.1.1"), Some(Ordering::Less));
        assert_eq!(compare_versions("0.1.0", "0.2.0"), Some(Ordering::Less));
        assert_eq!(compare_versions("0.1.0", "1.0.0"), Some(Ordering::Less));
        assert_eq!(compare_versions("v0.2.0", "0.1.0"), Some(Ordering::Greater));
        assert_eq!(compare_versions("0.1.0", "0.1.0"), Some(Ordering::Equal));
        assert_eq!(compare_versions("0.1", "0.1.0"), Some(Ordering::Equal));
        assert_eq!(
            compare_versions("0.1.0.1", "0.1.0"),
            Some(Ordering::Greater)
        );
        assert_eq!(compare_versions("1.0", "1.0.0.0"), Some(Ordering::Equal));
        assert_eq!(
            compare_versions(" 0.1.0 ", "0.1.0"),
            Some(Ordering::Equal),
            "首尾空白应被容忍（清单是人写的）"
        );
    }

    /// 不可比较的版本必须返回 None，**不许**退化成「相等」或「不更新」：
    /// 那会把「不知道」写成「已是最新」。
    #[test]
    fn unparsable_versions_are_none_never_false_by_default() {
        for bad in ["latest", "0.2.0-rc1", "1.x", "", "v", "1..0", "1.0.0+build"] {
            assert_eq!(
                compare_versions("0.1.0", bad),
                None,
                "{bad:?} 必须判为不可比较"
            );
        }
        assert_eq!(compare_versions("0.1.0", "99999999999999999999999"), None);
        assert_eq!(compare_versions("not-a-version", "0.1.0"), None);
    }

    /// JSONL 记录往返回来的字段必须逐字一致 —— 历史接口列出的东西
    /// 就是 prepare 当场写下的东西。
    #[test]
    fn history_record_round_trips_through_jsonl() {
        let rec = HistoryRecord {
            at_unix: 1_759_851_234,
            app_version: "0.1.0".into(),
            backup_name: "pre-upgrade-1759851234000".into(),
            backup_dir: "/srv/quill/backups/pre-upgrade-1759851234000".into(),
            db_sha256: "ab".repeat(32),
            files: 3,
            total_bytes: 12345,
        };
        let line = serde_json::to_string(&rec).expect("序列化");
        assert!(!line.contains('\n'), "JSONL 一行就是一条记录");
        let back: HistoryRecord = serde_json::from_str(&line).expect("反序列化");
        assert_eq!(back.at_unix, rec.at_unix);
        assert_eq!(back.backup_dir, rec.backup_dir);
        assert_eq!(back.db_sha256, rec.db_sha256);
        assert_eq!(back.files, rec.files);
        assert_eq!(back.total_bytes, rec.total_bytes);
    }

    /// 名字由服务端「时间戳 + 碰撞后缀」组成，绝不含用户的输入；
    /// 占位靠 `create_dir` 的原子性 —— 并发下两个请求不可能拿到同一个名字。
    #[test]
    fn backup_dir_reservation_is_atomic_and_avoids_existing_names() {
        let root = std::env::temp_dir().join(format!(
            "quill-upgrade-name-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&root);

        let (first, first_dir) = reserve_backup_dir(&root).expect("第一次预留");
        assert!(
            first_dir.is_dir(),
            "预留必须真把目录建出来（create_dir 原子占位，不是先查后建）"
        );
        let (second, second_dir) = reserve_backup_dir(&root).expect("第二次预留");
        assert_ne!(first, second, "已占用的名字必须被避开");
        assert!(second_dir.is_dir());

        for name in [&first, &second] {
            assert!(name.starts_with("pre-upgrade-"), "{name}");
            assert!(
                name.chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
                "名字里不该出现路径分隔符等字符：{name}"
            );
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn sha_helper_and_manifest_name_are_wired_from_quill_backup() {
        // 这条测试是「响应字段来自真实文件」的最小自检：摘要助手确实能读到内容，
        // 而且清单文件名与 quill-backup 是同一个常量（写错名字会让 files 校验永远失败）。
        let dir = std::env::temp_dir().join(format!(
            "quill-upgrade-digest-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("建临时目录");
        let f = dir.join("probe.bin");
        std::fs::write(&f, b"quill").expect("写探针文件");
        assert_eq!(
            sha256_file(&f).expect("重算摘要"),
            quill_backup::sha256_bytes(b"quill"),
            "磁盘文件的摘要必须与同一份字节在内存里算出来的一致"
        );
        assert_eq!(MANIFEST_NAME, "MANIFEST");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
