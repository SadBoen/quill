use std::sync::Arc;

use quill_adapters::{ExpertId, SessionId, UserId};
use quill_server::db::{storage_error, DbBridge};

#[derive(Debug, Clone)]
pub struct Fixture {
    pub team_id: [u8; 16],

    pub leader_session: SessionId,

    pub room_id: String,
}

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
