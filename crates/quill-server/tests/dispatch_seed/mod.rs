//! 派工测试的**外键前提**夹具。
//!
//! # 它替谁干活
//!
//! `task_dispatches` 的 `team_id` / `leader_session_id` / `member_session_id`
//! 是 `NOT NULL` 外键（`PRAGMA foreign_keys = ON` 由 `quill-store` 强制），
//! 而真正写 `users` / `sessions` / `teams` / `team_members` 的仓储**尚未实现**
//! （`quill-control` / 会话仓储都还是骨架）。
//!
//! 于是本模块用**裸 SQL** 建出这四张表里派工所依赖的最小前提行。
//! 它不是「伪造业务数据」，而是明确标注的**外键前提**：
//! 用户、会话、团队都是真实行，且满足表上全部 CHECK / 复合外键。
//!
//! # 建行顺序为什么是这个顺序（不能随手改）
//!
//! `sessions` 的 CHECK 要求 `team_leader` / `team_member` 必须带 `team_id`，
//! 而 `teams` 又外键引用 `sessions(user_id, id)` —— **互相依赖**。
//! 唯一能同时满足两者的顺序是：
//!
//! 1. `users`
//! 2. `sessions`（先以 `kind='solo'` 插入：solo 不要求 `team_id`）
//! 3. `teams`（此时 `leader_session_id` 的外键已满足）
//! 4. `UPDATE sessions` 把主持人改成 `team_leader`、成员改成 `team_member`
//! 5. `team_members`
//!
//! 少任何一步，`PRAGMA foreign_keys=ON` 会当场拒绝写入 —— 那正是我们要的：
//! 前提没建成就红，而不是「跳过」。

use std::sync::Arc;

use quill_adapters::{ExpertId, SessionId, UserId};
use quill_server::db::{storage_error, DbBridge};

/// 派工夹具的标识。
///
/// ⚠️ 三个字段都**必须**被调用方真的用上（团队填外键列、主持人会话填
/// `leader_session_id`、房间填 `room_id`）—— 这不是洁癖：
/// 若某个字段没人读，它就是「测试里随手编的假数据」的入口，
/// 而派工的真实语义（房间 + 轮次 + 成员）正建立在这三者之上。
#[derive(Debug, Clone)]
pub struct Fixture {
    /// 团队标识（16 字节，对应 `task_dispatches.team_id`）。
    pub team_id: [u8; 16],
    /// 主持人会话（对应 `task_dispatches.leader_session_id`）。
    pub leader_session: SessionId,
    /// 房间标识。
    pub room_id: String,
}

/// 由「团队种子 + 专家名」派生一条**确定**的成员会话标识。
///
/// ⚠️ 确定性是刻意的：同一专家在两次夹具构建里必须拿到同一个会话 id，
/// 否则「重启后仍能记账」这类用例会因为换了 id 而变成另一条记录。
///
/// ⚠️ **公开**它是为了让用例不要再自己抄一份派生逻辑：
/// 两份实现一旦漂移，症状是「外键莫名其妙失败」，排障方向会被带偏。
pub fn member_session(team_seed: u8, expert: &ExpertId) -> SessionId {
    let mut bytes = [0u8; 16];
    bytes[0] = team_seed;
    bytes[1] = 0xA0;
    for (i, b) in expert.as_str().bytes().take(14).enumerate() {
        bytes[2 + i] = b;
    }
    SessionId::from_bytes(bytes)
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// 建出派工所需的全套前提行。
///
/// `members` 里的每个专家都会得到一条 `team_members` 行与一条成员会话。
pub fn seed(db: &Arc<DbBridge>, owner: UserId, team_seed: u8, members: &[&str]) -> Fixture {
    let room_id = format!("room-{}", team_seed);
    let team_id = [team_seed; 16];
    let leader = SessionId::from_bytes([team_seed; 16]);
    let now = now_ms();

    let owner_blob = owner.as_bytes().to_vec();
    let leader_blob = leader.as_bytes().to_vec();
    let experts: Vec<ExpertId> = members
        .iter()
        .map(|m| ExpertId::parse(m).unwrap_or_else(|e| panic!("测试用专家名 {m:?} 非法：{e}")))
        .collect();
    let member_sessions: Vec<SessionId> = experts
        .iter()
        .map(|e| member_session(team_seed, e))
        .collect();

    // ① users
    // ⚠️ `password_algo` **刻意不给默认值**（schema 注释已说明理由：
    //    默认值写成未实现的算法会让用户永远登不进去且不报错）。
    //    因此夹具必须显式写它，取 `quill-control` 的真实取值形态。
    let user_name = format!("user-{team_seed}");
    db.call({
        let sql = "INSERT INTO users (id, username, username_norm, display_name, \
                    password_hash, password_salt, password_algo, role, pwd_changed_at, \
                    created_at, updated_at) \
                    VALUES (?, ?, ?, ?, ?, ?, 'pbkdf2-hmac-sha256$i=600000', 'owner', ?, ?, ?)"
            .to_string();
        let name = user_name.clone();
        let blob = owner_blob.clone();
        move |pool, _rt| {
            Box::pin(async move {
                sqlx::query(&sql)
                    .bind(blob)
                    .bind(&name)
                    .bind(&name)
                    .bind("测试用户")
                    .bind(vec![0u8; 32])
                    .bind(vec![0u8; 16])
                    .bind(now)
                    .bind(now)
                    .bind(now)
                    .execute(&pool)
                    .await
                    .map_err(|e| storage_error("插入测试用户", e))?;
                Ok(())
            })
        }
    })
    .unwrap_or_else(|e| panic!("插入测试用户失败：{e}"));

    // ② sessions：先以 solo 插入（solo 不要求 team_id，绕开与 teams 的循环依赖）
    for (i, sid) in std::iter::once(leader)
        .chain(member_sessions.iter().copied())
        .enumerate()
    {
        db.call({
            let sql = "INSERT INTO sessions (user_id, id, kind, room_id, expert_id, \
                        workspace_path, created_at, updated_at, last_active_at) \
                        VALUES (?, ?, 'solo', ?, NULL, ?, ?, ?, ?)"
                .to_string();
            let blob = owner_blob.clone();
            let sblob = sid.as_bytes().to_vec();
            let room = room_id.clone();
            move |pool, _rt| {
                Box::pin(async move {
                    sqlx::query(&sql)
                        .bind(blob)
                        .bind(sblob)
                        .bind(&room)
                        .bind(format!("data/quill-test/ws-{i}"))
                        .bind(now)
                        .bind(now)
                        .bind(now)
                        .execute(&pool)
                        .await
                        .map_err(|e| storage_error("插入测试会话", e))?;
                    Ok(())
                })
            }
        })
        .unwrap_or_else(|e| panic!("插入测试会话失败（第 {i} 条）：{e}"));
    }

    // ③ teams（此刻 leader 的外键已满足）
    db.call({
        let sql = "INSERT INTO teams (user_id, id, name, room_id, leader_session_id, \
                    leader_expert_id, state, state_changed_at, created_at, updated_at) \
                    VALUES (?, ?, ?, ?, ?, ?, 'IDLE', ?, ?, ?)"
            .to_string();
        let blob = owner_blob.clone();
        let tblob = team_id.to_vec();
        let lblob = leader_blob.clone();
        let room = room_id.clone();
        move |pool, _rt| {
            Box::pin(async move {
                sqlx::query(&sql)
                    .bind(blob)
                    .bind(tblob)
                    .bind(format!("team-{team_seed}"))
                    .bind(room)
                    .bind(lblob)
                    .bind("team-leader")
                    .bind(now)
                    .bind(now)
                    .bind(now)
                    .execute(&pool)
                    .await
                    .map_err(|e| storage_error("插入测试团队", e))?;
                Ok(())
            })
        }
    })
    .unwrap_or_else(|e| panic!("插入测试团队失败：{e}"));

    // ④ 主持人 → team_leader，成员 → team_member（带 team_id / parent_session_id / expert_id）
    db.call({
        let sql =
            "UPDATE sessions SET kind = 'team_leader', team_id = ?, expert_id = 'team-leader' \
                    WHERE user_id = ? AND id = ?"
                .to_string();
        let tblob = team_id.to_vec();
        let blob = owner_blob.clone();
        let lblob = leader_blob.clone();
        move |pool, _rt| {
            Box::pin(async move {
                sqlx::query(&sql)
                    .bind(tblob)
                    .bind(blob)
                    .bind(lblob)
                    .execute(&pool)
                    .await
                    .map_err(|e| storage_error("把主持人会话改为 team_leader", e))?;
                Ok(())
            })
        }
    })
    .unwrap_or_else(|e| panic!("更新主持人会话失败：{e}"));

    for (expert, sid) in experts.iter().zip(member_sessions.iter()) {
        db.call({
            let sql = "UPDATE sessions SET kind = 'team_member', team_id = ?, \
                        parent_session_id = ?, expert_id = ? WHERE user_id = ? AND id = ?"
                .to_string();
            let tblob = team_id.to_vec();
            let lblob = leader_blob.clone();
            let e2 = expert.as_str().to_string();
            let blob = owner_blob.clone();
            let sblob = sid.as_bytes().to_vec();
            move |pool, _rt| {
                Box::pin(async move {
                    sqlx::query(&sql)
                        .bind(tblob)
                        .bind(lblob)
                        .bind(e2)
                        .bind(blob)
                        .bind(sblob)
                        .execute(&pool)
                        .await
                        .map_err(|e| storage_error("把成员会话改为 team_member", e))?;
                    Ok(())
                })
            }
        })
        .unwrap_or_else(|e| panic!("更新成员会话失败（{expert}）：{e}"));
    }

    // ⑤ team_members（role='leader' 只能有一条，ux_team_leader 是部分唯一索引）
    for (idx, (expert, sid)) in experts.iter().zip(member_sessions.iter()).enumerate() {
        db.call({
            let sql = "INSERT INTO team_members (user_id, team_id, expert_id, role, \
                        member_session_id, state, state_changed_at, joined_at, created_at, updated_at) \
                        VALUES (?, ?, ?, 'member', ?, 'RUNNING', ?, ?, ?, ?)"
                .to_string();
            let tblob = team_id.to_vec();
            let e2 = expert.as_str().to_string();
            let sblob = sid.as_bytes().to_vec();
            let blob = owner_blob.clone();
            move |pool, _rt| {
                Box::pin(async move {
                    sqlx::query(&sql)
                        .bind(blob)
                        .bind(tblob)
                        .bind(e2)
                        .bind(sblob)
                        .bind(now)
                        .bind(now + idx as i64)
                        .bind(now)
                        .bind(now)
                        .execute(&pool)
                        .await
                        .map_err(|e| storage_error("插入团成员", e))?;
                    Ok(())
                })
            }
        })
        .unwrap_or_else(|e| panic!("插入团成员失败（第 {idx} 条）：{e}"));
    }

    Fixture {
        team_id,
        leader_session: leader,
        room_id,
    }
}
