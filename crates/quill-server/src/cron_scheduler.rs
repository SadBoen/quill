//! 定时任务的调度器（queue Q042）：**真会触发投递的那一半**。
//!
//! 投递走的是**与网页聊天同一条路**：`api_chat::{prepare_turn, run_turn, finish_turn}`。
//! 不另写一套「任务执行器」—— 那等于把对话循环复制一份，而两套迟早只改一边，
//! 「网页还能用、定时任务坏了」是最难发现的错法（与「通道」当初同一条取舍）。
//!
//! 两件事**刻意分开**：`run_due_once` 是纯函数式的「跑一轮」（测试直接喂一个固定的
//! `now` 调它，不等真实时钟、也不起后台任务），`spawn` 只是给它接上一个定时器。
//! 判据要能机器跑，就不能把被测逻辑埋在 `loop { sleep }` 里。

use std::time::Duration;

use quill_agent::AgentError;
use quill_core::turn::ReplyMode;
use quill_domain::{SessionId, UserId};

use crate::api_chat::{self, NullSink};
use crate::auth::{AuthContext, AuthUser};
use crate::chat_repo::{self, NewSoloSession};
use crate::cron_repo::{self, CronJobRow, Schedule};
use crate::state::AppState;

/// 后台循环的 tick 间隔。30 秒对「分钟级排期」够用，也是出错重试不至于太密的上限。
pub const TICK: Duration = Duration::from_secs(30);

/// 一条任务的下一次触发时间（epoch ms）。`at` 是一次性的 → `None`（投递后软删）。
fn next_after(schedule: Schedule, now: i64) -> Option<i64> {
    match schedule {
        Schedule::Every { every_seconds } => Some(now + every_seconds * 1000),
        Schedule::At { .. } => None,
    }
}

/// 跑一轮：把到点的任务投递掉。返回**真投递成功的条数** ——
/// 不是「取到几条」：两者差出来的就是失败的那几条（它们被 `mark_failed` 记了账）。
pub async fn run_due_once(state: &AppState, now: i64) -> Result<usize, AgentError> {
    let Some(db) = state.db.clone() else {
        return Ok(0);
    };
    let jobs = cron_repo::due(&db, now, cron_repo::DUE_BATCH)?;
    let mut delivered = 0usize;
    for (owner, job) in jobs {
        match deliver(state, owner, &job, now).await {
            Ok(()) => {
                delivered += 1;
                match next_after(job.schedule, now) {
                    Some(next) => cron_repo::mark_fired(&db, owner, job.id.clone(), now, next)?,
                    // 一次性任务投递完就软删：记录留着（排障要看），但它不该再出现在列表里。
                    None => {
                        cron_repo::soft_delete(&db, owner, job.id.clone(), now)?;
                    }
                }
            }
            Err(e) => {
                // **不静默**：原因写进 `last_error`，并把下次触发推迟到 5 分钟后（见
                // `cron_repo::mark_failed` 的注释：不推进会让用户以为「跳过了这一次」）。
                cron_repo::mark_failed(&db, owner, job.id.clone(), now, e.to_string())?;
            }
        }
    }
    Ok(delivered)
}

/// 把一条任务投递进它的会话：没有会话就先建一条，然后走与网页聊天同一个对话循环。
async fn deliver(
    state: &AppState,
    owner: UserId,
    job: &CronJobRow,
    now: i64,
) -> Result<(), AgentError> {
    let db = state.db.clone().ok_or_else(|| AgentError::Storage {
        detail: "存储不可用，无法投递定时任务。下一步：用 `quill doctor --section=db` 诊断。"
            .to_string(),
    })?;
    let sid = match job.session_id {
        Some(s) => s,
        None => {
            let s = ensure_job_session(&db, owner, job, now)?;
            cron_repo::set_session(&db, owner, job.id.clone(), s, now)?;
            s
        }
    };
    let user = AuthUser(AuthContext {
        user_id: owner,
        is_admin: false,
    });
    let mut prep = api_chat::prepare_turn(
        state.clone(),
        user,
        sid.to_compact_hex(),
        job.message.clone(),
    )
    .await
    .map_err(api_error)?;
    let outcome = api_chat::run_turn(&mut prep, ReplyMode::Once, &mut NullSink)
        .await
        .map_err(api_error)?;
    api_chat::finish_turn(&prep, outcome)
        .await
        .map_err(api_error)?;
    Ok(())
}

/// 建这条任务自己的会话。id 由 `owner + job.id` **确定性派生**：
/// 重试或换机重建时不会越建越多（同一个任务永远落回同一条会话）。
fn ensure_job_session(
    db: &crate::db::DbBridge,
    owner: UserId,
    job: &CronJobRow,
    now: i64,
) -> Result<SessionId, AgentError> {
    let raw = quill_core::digest::digest16("cron-session", &[owner.as_bytes(), job.id.as_bytes()]);
    if !chat_repo::session_exists(db, owner, raw)? {
        // `room_id` / `workspace_path` 的形状与 `api_chat` 建会话时一致
        // （`room-<12hex>` / `ws/<12hex>`）—— schema 要求 workspace_path 是 1~512 字符
        // 且不含 `..`，给空串会被 CHECK 直接拒掉。
        let short: String = raw.iter().take(6).map(|b| format!("{b:02x}")).collect();
        chat_repo::insert_solo_session(
            db,
            owner,
            NewSoloSession {
                id: raw,
                room_id: format!("room-cron-{short}"),
                title: job.name.clone(),
                model: String::new(),
                workspace_path: format!("ws/cron-{short}"),
                now,
            },
            "建定时任务会话",
        )?;
    }
    Ok(SessionId::from_bytes(raw))
}

/// `ApiError` → `AgentError`：调度器没有 HTTP 响应可回，但**错误内容一个字不丢** ——
/// `detail` 与「下一步」都会被写进 `last_error`，排障时能看到真正的原因。
fn api_error(e: crate::error::ApiError) -> AgentError {
    AgentError::Storage {
        detail: format!("{}；{}", e.detail(), e.next_step()),
    }
}

/// 起后台循环（只在服务启动时调一次）。
pub fn spawn(state: AppState) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(TICK);
        // `interval` 的第一跳是「立刻」，跳过它：启动那一瞬间不该把所有到期任务
        // 一起打起来（冷启动时数据库刚打开，正是最忙的时候）。
        ticker.tick().await;
        loop {
            ticker.tick().await;
            let now = crate::db::now_ms();
            match run_due_once(&state, now).await {
                Ok(0) => {}
                Ok(n) => eprintln!("[cron] 投递了 {n} 条定时任务"),
                Err(e) => eprintln!("[cron] 这一轮调度失败：{e}"),
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_reschedules_from_now_and_at_does_not() {
        assert_eq!(
            next_after(Schedule::Every { every_seconds: 60 }, 1_000),
            Some(61_000)
        );
        // 一次性任务没有「下一次」——调用方据此软删。
        assert_eq!(next_after(Schedule::At { run_at: 5_000 }, 1_000), None);
    }
}
