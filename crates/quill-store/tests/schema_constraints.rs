use sqlx::error::{DatabaseError, ErrorKind};
use sqlx::{Row, SqlitePool};

fn constraint_kind(e: &dyn DatabaseError) -> Option<ErrorKind> {
    match e.kind() {
        ErrorKind::CheckViolation
        | ErrorKind::UniqueViolation
        | ErrorKind::ForeignKeyViolation
        | ErrorKind::NotNullViolation => Some(e.kind()),
        _ => None,
    }
}

type ExecResult = Result<sqlx::sqlite::SqliteQueryResult, sqlx::Error>;

async fn expect_constraint<F, Fut>(case: &str, f: F)
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = ExecResult>,
{
    match f().await {
        Ok(_) => panic!("❌ [{case}] 语句被【接受】了 —— 预期应因约束被拒绝"),
        Err(sqlx::Error::Database(ref e)) => match constraint_kind(&**e) {
            Some(_) => {}
            None => panic!(
                "🚨 [{case}] 语句因【非约束原因】失败 —— 这是装置失效，不是约束生效！\n\
                 若把它当成「约束生效」就是假绿（第 1 类假闸门）。\n错误: {e}"
            ),
        },
        Err(other) => panic!("🚨 [{case}] 语句因【非数据库错误】失败 —— 装置失效：{other}"),
    }
}

async fn assert_legal<F, Fut>(case: &str, f: F)
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = ExecResult>,
{
    if let Err(e) = f().await {
        panic!(
            "🚨 [{case}] 合法数据竟然执行失败 —— 后续所有「约束生效」断言都将失效！\n\
             （这正是 Python 版假绿第 2/3 类的成因）\n错误: {e}"
        );
    }
}

const MIGRATION: &str = include_str!("../migrations/0001_init.sql");

fn id(n: u8) -> Vec<u8> {
    let mut v = vec![0u8; 16];
    v[0] = n;
    v[15] = 0xA0;
    v
}

fn h32(n: u8) -> Vec<u8> {
    vec![n; 32]
}

const NOW: i64 = 1_700_000_000_000;

const ALGO: &str = "pbkdf2-hmac-sha256$i=600000";

async fn fresh_pool() -> SqlitePool {
    let pool = quill_store::in_memory().await.expect("建内存库");
    quill_store::run_migration(&pool, MIGRATION)
        .await
        .expect("跑迁移");
    pool
}

async fn seed_two_users(pool: &SqlitePool) {
    for (n, name, role) in [(1u8, "alice", "owner"), (2u8, "bob", "member")] {
        sqlx::query(
            "INSERT INTO users(id,username,username_norm,display_name,password_hash,\
             password_salt,password_algo,role,pwd_changed_at,created_at,updated_at)\
             VALUES(?,?,?,?,?,?,?,?,?,?,?)",
        )
        .bind(id(n))
        .bind(name)
        .bind(name)
        .bind(name)
        .bind(h32(n))
        .bind(vec![7u8; 16])
        .bind(ALGO)
        .bind(role)
        .bind(NOW)
        .bind(NOW)
        .bind(NOW)
        .execute(pool)
        .await
        .expect("种用户");
    }
    for (uid, sid, who) in [(1u8, 0x11u8, "alice"), (2u8, 0x22u8, "bob")] {
        sqlx::query(
            "INSERT INTO sessions(user_id,id,kind,room_id,workspace_path,created_at,updated_at,\
             last_active_at) VALUES(?,?,?,?,?,?,?,?)",
        )
        .bind(id(uid))
        .bind(id(sid))
        .bind("solo")
        .bind(format!("leader_session:{who}:{sid:02x}"))
        .bind(format!("sessions/{sid:02x}"))
        .bind(NOW)
        .bind(NOW)
        .bind(NOW)
        .execute(pool)
        .await
        .expect("种 session");
    }
}

async fn seed_team_a(pool: &SqlitePool) {
    sqlx::query(
        "INSERT INTO teams(user_id,id,name,room_id,leader_session_id,leader_expert_id,\
         state_changed_at,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?,?)",
    )
    .bind(id(1))
    .bind(id(0x41))
    .bind("A队")
    .bind("roomA")
    .bind(id(0x11))
    .bind("lead")
    .bind(NOW)
    .bind(NOW)
    .bind(NOW)
    .execute(pool)
    .await
    .expect("种 team");
}

const MSG: &str = "INSERT INTO messages(user_id,id,session_id,seq,role,content,created_at)\
                   VALUES(?,?,?,?,?,?,?)";
const TEAM_INS: &str =
    "INSERT INTO teams(user_id,id,name,room_id,leader_session_id,leader_expert_id,\
                        state_changed_at,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?,?)";
const TM: &str = "INSERT INTO team_members(user_id,team_id,expert_id,role,member_session_id,state,\
                  state_changed_at,joined_at,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?,?,?)";
const DISP: &str =
    "INSERT INTO task_dispatches(user_id,id,room_id,team_id,round,leader_session_id,\
                    member_session_id,member_expert_id,task_digest,state,dispatched_at,created_at,\
                    updated_at) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?)";
const DISP_T: &str = "INSERT INTO task_dispatches(user_id,id,room_id,team_id,round,leader_session_id,\
                      member_session_id,member_expert_id,task_digest,state,dispatched_at,settled_at,\
                      created_at,updated_at) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?)";
const EXPERT: &str =
    "INSERT INTO experts(id,owner_user_id,display_name,version,description,visibility,\
                      tool_policy_json,license,asset_hash,persona_hash,created_at,updated_at)\
                      VALUES(?,?,?,?,?,?,?,?,?,?,?,?)";
const SESS: &str = "INSERT INTO sessions(user_id,id,kind,room_id,workspace_path,created_at,\
                    updated_at,last_active_at) VALUES(?,?,?,?,?,?,?,?)";
const WIKI: &str = "INSERT INTO wiki_index(user_id,term,doc_id,term_kind,tf,field_len,rel_path,\
                    page_title,content_hash,bytes,indexed_at) VALUES(?,?,?,?,?,?,?,?,?,?,?)";
const INV: &str =
    "INSERT INTO invites(id,code_hash,created_by,expires_at,created_at) VALUES(?,?,?,?,?)";

#[tokio::test]
async fn password_algo_has_no_lying_default() {
    let pool = fresh_pool().await;

    let r = sqlx::query(
        "INSERT INTO users(id,username,username_norm,display_name,password_hash,\
         password_salt,role,pwd_changed_at,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?,?,?)",
    )
    .bind(id(1))
    .bind("alice")
    .bind("alice")
    .bind("Alice")
    .bind(h32(1))
    .bind(vec![7u8; 16])
    .bind("owner")
    .bind(NOW)
    .bind(NOW)
    .bind(NOW)
    .execute(&pool)
    .await;
    assert!(
        r.is_err(),
        "🚨 漏写 password_algo 竟然插入成功了 —— 说明有人把 `DEFAULT 'argon2id'` 加回来了。\n\
         那个默认值与实现（PBKDF2）不符，会静默产生永远无法登录的账号。"
    );

    expect_constraint("password_algo 空串", || async {
        sqlx::query(
            "INSERT INTO users(id,username,username_norm,display_name,password_hash,\
             password_salt,password_algo,role,pwd_changed_at,created_at,updated_at)\
             VALUES(?,?,?,?,?,?,?,?,?,?,?)",
        )
        .bind(id(1))
        .bind("alice")
        .bind("alice")
        .bind("Alice")
        .bind(h32(1))
        .bind(vec![7u8; 16])
        .bind("")
        .bind("owner")
        .bind(NOW)
        .bind(NOW)
        .bind(NOW)
        .execute(&pool)
        .await
    })
    .await;

    assert_legal("显式 password_algo", || async {
        sqlx::query(
            "INSERT INTO users(id,username,username_norm,display_name,password_hash,\
             password_salt,password_algo,role,pwd_changed_at,created_at,updated_at)\
             VALUES(?,?,?,?,?,?,?,?,?,?,?)",
        )
        .bind(id(1))
        .bind("alice")
        .bind("alice")
        .bind("Alice")
        .bind(h32(1))
        .bind(vec![7u8; 16])
        .bind(ALGO)
        .bind("owner")
        .bind(NOW)
        .bind(NOW)
        .bind(NOW)
        .execute(&pool)
        .await
    })
    .await;
}

#[tokio::test]
async fn a1_cross_user_foreign_keys_all_blocked() {
    let pool = fresh_pool().await;
    seed_two_users(&pool).await;

    assert_legal("A1 前置：合法消息", || async {
        sqlx::query(MSG)
            .bind(id(1))
            .bind(id(0x31))
            .bind(id(0x11))
            .bind(1i64)
            .bind("user")
            .bind("hi")
            .bind(NOW)
            .execute(&pool)
            .await
    })
    .await;

    expect_constraint("A→B 的 session 写消息", || async {
        sqlx::query(MSG)
            .bind(id(1))
            .bind(id(0x32))
            .bind(id(0x22))
            .bind(1i64)
            .bind("user")
            .bind("x")
            .bind(NOW)
            .execute(&pool)
            .await
    })
    .await;

    seed_team_a(&pool).await;

    expect_constraint("A 的 team 挂 B 的 session 为 leader", || async {
        sqlx::query(TEAM_INS)
            .bind(id(1))
            .bind(id(0x42))
            .bind("x")
            .bind("r")
            .bind(id(0x22))
            .bind("e")
            .bind(NOW)
            .bind(NOW)
            .bind(NOW)
            .execute(&pool)
            .await
    })
    .await;

    expect_constraint("A 的 team_members 挂 B 的 session", || async {
        sqlx::query(TM)
            .bind(id(1))
            .bind(id(0x41))
            .bind("e1")
            .bind("member")
            .bind(id(0x22))
            .bind("IDLE")
            .bind(NOW)
            .bind(NOW)
            .bind(NOW)
            .bind(NOW)
            .execute(&pool)
            .await
    })
    .await;

    expect_constraint("A 的派工挂 B 的 member session", || async {
        sqlx::query(DISP)
            .bind(id(1))
            .bind(id(0x51))
            .bind("roomA")
            .bind(id(0x41))
            .bind(0i64)
            .bind(id(0x11))
            .bind(id(0x22))
            .bind("e1")
            .bind(h32(1))
            .bind("RUNNING")
            .bind(NOW)
            .bind(NOW)
            .bind(NOW)
            .execute(&pool)
            .await
    })
    .await;
}

#[tokio::test]
async fn a2_delete_user_cascades_only_own_rows() {
    let pool = fresh_pool().await;
    seed_two_users(&pool).await;

    for n in [1u8, 2] {
        sqlx::query(WIKI)
            .bind(id(n))
            .bind("rust")
            .bind(id(0x60 + n))
            .bind(0i64)
            .bind(3i64)
            .bind(100i64)
            .bind(format!("{n}.md"))
            .bind("T")
            .bind(h32(n))
            .bind(900i64)
            .bind(NOW)
            .execute(&pool)
            .await
            .expect("种 wiki_index");
    }

    sqlx::query("DELETE FROM users WHERE id=?")
        .bind(id(2))
        .execute(&pool)
        .await
        .expect("删 B");

    let a_rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM wiki_index WHERE user_id=?")
        .bind(id(1))
        .fetch_one(&pool)
        .await
        .expect("查 A");
    assert_eq!(a_rows, 1, "删 B 影响了 A 的索引行");
    let b_rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM wiki_index WHERE user_id=?")
        .bind(id(2))
        .fetch_one(&pool)
        .await
        .expect("查 B");
    assert_eq!(b_rows, 0, "删 B 未级联清空其索引行");
}

#[tokio::test]
async fn b1_expert_slug_rejects_traversal() {
    let pool = fresh_pool().await;
    seed_two_users(&pool).await;

    assert_legal("B1 前置：合法 slug", || async {
        sqlx::query(EXPERT)
            .bind("ops-engineer")
            .bind(id(1))
            .bind("n")
            .bind("1")
            .bind("d")
            .bind("user_authored")
            .bind("{}")
            .bind("MIT")
            .bind(h32(1))
            .bind(h32(2))
            .bind(NOW)
            .bind(NOW)
            .execute(&pool)
            .await
    })
    .await;

    for bad in ["../etc", "a/b", "a\\b", "Foo", "a..b", "", "a b", "."] {
        expect_constraint(&format!("专家 id={bad:?}"), || async {
            sqlx::query(EXPERT)
                .bind(bad)
                .bind(id(1))
                .bind("n")
                .bind("1")
                .bind("d")
                .bind("user_authored")
                .bind("{}")
                .bind("MIT")
                .bind(h32(1))
                .bind(h32(2))
                .bind(NOW)
                .bind(NOW)
                .execute(&pool)
                .await
        })
        .await;
    }
}

#[tokio::test]
async fn b2_builtin_expert_owner_must_be_zero_uuid() {
    let pool = fresh_pool().await;
    seed_two_users(&pool).await;
    expect_constraint("is_builtin=0 却用全零 owner UUID", || async {
        sqlx::query(EXPERT)
            .bind("x1")
            .bind(vec![0u8; 16])
            .bind("n")
            .bind("1")
            .bind("d")
            .bind("builtin_system")
            .bind("{}")
            .bind("MIT")
            .bind(h32(1))
            .bind(h32(2))
            .bind(NOW)
            .bind(NOW)
            .execute(&pool)
            .await
    })
    .await;
}

#[tokio::test]
async fn b3_tool_policy_required_tp1() {
    let pool = fresh_pool().await;
    seed_two_users(&pool).await;

    let res = sqlx::query("INSERT INTO experts(id,owner_user_id,display_name,version,description,\
                          visibility,tool_policy_json,license,asset_hash,persona_hash,created_at,updated_at)\
                          VALUES(?,?,?,?,?,?,NULL,?,?,?,?,?)")
        .bind("x2").bind(id(1)).bind("n").bind("1").bind("d")
        .bind("user_authored").bind("MIT")
        .bind(h32(1)).bind(h32(2)).bind(NOW).bind(NOW)
        .execute(&pool).await;
    match res {
        Ok(_) => panic!("❌ tool_policy 为 NULL 竟被接受 —— TP-1 约束未生效"),
        Err(sqlx::Error::Database(ref e)) => {
            assert!(
                constraint_kind(&**e).is_some(),
                "🚨 非约束错误，装置失效：{e}"
            );
        }
        Err(o) => panic!("🚨 非数据库错误：{o}"),
    }
}

#[tokio::test]
async fn b4_workspace_path_rejects_traversal() {
    let pool = fresh_pool().await;
    seed_two_users(&pool).await;
    for (case, path) in [
        ("绝对路径", "/etc/passwd"),
        ("目录穿越", "../../B/sessions/x"),
    ] {
        expect_constraint(case, || async {
            sqlx::query(SESS)
                .bind(id(1))
                .bind(id(0x91))
                .bind("solo")
                .bind("r")
                .bind(path)
                .bind(NOW)
                .bind(NOW)
                .bind(NOW)
                .execute(&pool)
                .await
        })
        .await;
    }
}

#[tokio::test]
async fn b5_wiki_rel_path_rejects_traversal() {
    let pool = fresh_pool().await;
    seed_two_users(&pool).await;
    expect_constraint("wiki_index rel_path 含 ..", || async {
        sqlx::query(WIKI)
            .bind(id(1))
            .bind("t")
            .bind(id(0x42))
            .bind(0i64)
            .bind(1i64)
            .bind(5i64)
            .bind("../B/secret.md")
            .bind("T")
            .bind(h32(1))
            .bind(10i64)
            .bind(NOW)
            .execute(&pool)
            .await
    })
    .await;
}

#[tokio::test]
async fn c1_dispatch_state_machine_invariants() {
    let pool = fresh_pool().await;
    seed_two_users(&pool).await;
    seed_team_a(&pool).await;

    assert_legal("C1 前置：合法派工", || async {
        sqlx::query(DISP)
            .bind(id(1))
            .bind(id(0x61))
            .bind("roomA")
            .bind(id(0x41))
            .bind(0i64)
            .bind(id(0x11))
            .bind(id(0x11))
            .bind("e1")
            .bind(h32(1))
            .bind("RUNNING")
            .bind(NOW)
            .bind(NOW)
            .bind(NOW)
            .execute(&pool)
            .await
    })
    .await;

    expect_constraint("DONE 但 settled_at 为空", || async {
        sqlx::query(DISP)
            .bind(id(1))
            .bind(id(0x62))
            .bind("roomA")
            .bind(id(0x41))
            .bind(0i64)
            .bind(id(0x11))
            .bind(id(0x11))
            .bind("e1")
            .bind(h32(1))
            .bind("DONE")
            .bind(NOW)
            .bind(NOW)
            .bind(NOW)
            .execute(&pool)
            .await
    })
    .await;

    expect_constraint("RUNNING 却带 settled_at", || async {
        sqlx::query(DISP_T)
            .bind(id(1))
            .bind(id(0x63))
            .bind("roomA")
            .bind(id(0x41))
            .bind(0i64)
            .bind(id(0x11))
            .bind(id(0x11))
            .bind("e1")
            .bind(h32(1))
            .bind("RUNNING")
            .bind(NOW)
            .bind(NOW)
            .bind(NOW)
            .bind(NOW)
            .execute(&pool)
            .await
    })
    .await;

    expect_constraint("非法状态 PANICKED", || async {
        sqlx::query(DISP)
            .bind(id(1))
            .bind(id(0x64))
            .bind("roomA")
            .bind(id(0x41))
            .bind(0i64)
            .bind(id(0x11))
            .bind(id(0x11))
            .bind("e1")
            .bind(h32(1))
            .bind("PANICKED")
            .bind(NOW)
            .bind(NOW)
            .bind(NOW)
            .execute(&pool)
            .await
    })
    .await;

    expect_constraint("同轮同成员二次派工", || async {
        sqlx::query(DISP)
            .bind(id(1))
            .bind(id(0x65))
            .bind("roomA")
            .bind(id(0x41))
            .bind(0i64)
            .bind(id(0x11))
            .bind(id(0x11))
            .bind("e1")
            .bind(h32(1))
            .bind("RUNNING")
            .bind(NOW)
            .bind(NOW)
            .bind(NOW)
            .execute(&pool)
            .await
    })
    .await;
}

#[tokio::test]
async fn c2_team_and_member_invariants() {
    let pool = fresh_pool().await;
    seed_two_users(&pool).await;
    seed_team_a(&pool).await;

    assert_legal("C2 前置：leader 成员", || async {
        sqlx::query(TM)
            .bind(id(1))
            .bind(id(0x41))
            .bind("lead")
            .bind("leader")
            .bind(Option::<Vec<u8>>::None)
            .bind("IDLE")
            .bind(NOW)
            .bind(NOW)
            .bind(NOW)
            .bind(NOW)
            .execute(&pool)
            .await
    })
    .await;

    expect_constraint("同一 team 第二个 leader", || async {
        sqlx::query(TM)
            .bind(id(1))
            .bind(id(0x41))
            .bind("e2")
            .bind("leader")
            .bind(Option::<Vec<u8>>::None)
            .bind("IDLE")
            .bind(NOW)
            .bind(NOW)
            .bind(NOW)
            .bind(NOW)
            .execute(&pool)
            .await
    })
    .await;

    expect_constraint("team_members 有 session 却 IDLE", || async {
        sqlx::query(TM)
            .bind(id(1))
            .bind(id(0x41))
            .bind("e3")
            .bind("member")
            .bind(id(0x11))
            .bind("IDLE")
            .bind(NOW)
            .bind(NOW)
            .bind(NOW)
            .bind(NOW)
            .execute(&pool)
            .await
    })
    .await;
}

#[tokio::test]
async fn c3_inv7_limits_enforced() {
    let pool = fresh_pool().await;
    seed_two_users(&pool).await;
    seed_team_a(&pool).await;

    assert_legal("C3 前置：合法上限值", || async {
        sqlx::query("UPDATE teams SET max_dispatch=4, max_replan=2 WHERE user_id=? AND id=?")
            .bind(id(1))
            .bind(id(0x41))
            .execute(&pool)
            .await
    })
    .await;

    for (case, sql, val) in [
        (
            "max_dispatch>8",
            "UPDATE teams SET max_dispatch=? WHERE user_id=? AND id=?",
            99i64,
        ),
        (
            "max_dispatch<2",
            "UPDATE teams SET max_dispatch=? WHERE user_id=? AND id=?",
            1i64,
        ),
        (
            "max_replan>5",
            "UPDATE teams SET max_replan=? WHERE user_id=? AND id=?",
            99i64,
        ),
        (
            "max_ask_depth>5",
            "UPDATE teams SET max_ask_depth=? WHERE user_id=? AND id=?",
            99i64,
        ),
    ] {
        expect_constraint(case, || async {
            sqlx::query(sql)
                .bind(val)
                .bind(id(1))
                .bind(id(0x41))
                .execute(&pool)
                .await
        })
        .await;
    }
}

#[tokio::test]
async fn d1_message_seq_unique_per_session() {
    let pool = fresh_pool().await;
    seed_two_users(&pool).await;

    assert_legal("D1 前置：A 的 seq=1", || async {
        sqlx::query(MSG)
            .bind(id(1))
            .bind(id(0x71))
            .bind(id(0x11))
            .bind(1i64)
            .bind("user")
            .bind("hi")
            .bind(NOW)
            .execute(&pool)
            .await
    })
    .await;
    expect_constraint("同 session 重复 seq=1", || async {
        sqlx::query(MSG)
            .bind(id(1))
            .bind(id(0x72))
            .bind(id(0x11))
            .bind(1i64)
            .bind("user")
            .bind("dup")
            .bind(NOW)
            .execute(&pool)
            .await
    })
    .await;

    assert_legal("B 的 seq=1（不同 session）", || async {
        sqlx::query(MSG)
            .bind(id(2))
            .bind(id(0x73))
            .bind(id(0x22))
            .bind(1i64)
            .bind("user")
            .bind("hi")
            .bind(NOW)
            .execute(&pool)
            .await
    })
    .await;
}

#[tokio::test]
async fn e1_soft_delete_allows_username_reuse() {
    let pool = fresh_pool().await;
    seed_two_users(&pool).await;

    sqlx::query("UPDATE users SET deleted_at=? WHERE id=?")
        .bind(NOW)
        .bind(id(1))
        .execute(&pool)
        .await
        .expect("软删 A");

    assert_legal(
        "E1：软删后他人【存活】占用同一用户名",
        || async {
            sqlx::query(
            "INSERT INTO users(id,username,username_norm,display_name,password_hash,password_salt,\
             password_algo,role,pwd_changed_at,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?,?,?,?)",
        )
        .bind(id(9)).bind("alice2").bind("alice").bind("X")
        .bind(h32(1)).bind(vec![7u8; 16]).bind(ALGO).bind("member")
        .bind(NOW).bind(NOW).bind(NOW)
        .execute(&pool).await
        },
    )
    .await;

    let res = sqlx::query("UPDATE users SET deleted_at=NULL WHERE id=?")
        .bind(id(1))
        .execute(&pool)
        .await;
    match res {
        Ok(_) => panic!("❌ A 恢复成功 —— 部分唯一索引未生效"),
        Err(sqlx::Error::Database(ref e)) => {
            assert!(
                constraint_kind(&**e).is_some(),
                "🚨 非约束错误，装置失效：{e}"
            );
        }
        Err(o) => panic!("🚨 非数据库错误：{o}"),
    }

    sqlx::query("UPDATE users SET deleted_at=? WHERE id=?")
        .bind(NOW)
        .bind(id(9))
        .execute(&pool)
        .await
        .expect("软删 X");
    sqlx::query("UPDATE users SET deleted_at=NULL WHERE id=?")
        .bind(id(1))
        .execute(&pool)
        .await
        .expect("X 软删后 A 应能恢复 —— 索引方向反了？");
}

#[tokio::test]
async fn f1_invites_structure_ready_for_v11() {
    let pool = fresh_pool().await;
    seed_two_users(&pool).await;

    assert_legal("F1：invites 合法行", || async {
        sqlx::query(INV)
            .bind(id(0x81))
            .bind(h32(1))
            .bind(id(1))
            .bind(NOW + 86_400_000)
            .bind(NOW)
            .execute(&pool)
            .await
    })
    .await;

    expect_constraint("invites code_hash 长度错", || async {
        sqlx::query(INV)
            .bind(id(0x82))
            .bind(vec![9u8; 48])
            .bind(id(1))
            .bind(NOW + 86_400_000)
            .bind(NOW)
            .execute(&pool)
            .await
    })
    .await;

    expect_constraint("invites code_hash 重复", || async {
        sqlx::query(INV)
            .bind(id(0x83))
            .bind(h32(1))
            .bind(id(1))
            .bind(NOW + 86_400_000)
            .bind(NOW)
            .execute(&pool)
            .await
    })
    .await;

    expect_constraint("invites 签发人不存在", || async {
        sqlx::query(INV)
            .bind(id(0x84))
            .bind(h32(3))
            .bind(id(0x7f))
            .bind(NOW + 86_400_000)
            .bind(NOW)
            .execute(&pool)
            .await
    })
    .await;

    expect_constraint("invites 出生即过期", || async {
        sqlx::query(INV)
            .bind(id(0x85))
            .bind(h32(4))
            .bind(id(1))
            .bind(NOW)
            .bind(NOW)
            .execute(&pool)
            .await
    })
    .await;

    expect_constraint("invites used_count>max_uses", || async {
        sqlx::query(
            "INSERT INTO invites(id,code_hash,created_by,max_uses,used_count,expires_at,created_at)\
             VALUES(?,?,?,?,?,?,?)",
        )
        .bind(id(0x86)).bind(h32(5)).bind(id(1)).bind(1i64).bind(5i64)
        .bind(NOW + 86_400_000).bind(NOW)
        .execute(&pool).await
    })
    .await;
}

#[tokio::test]
async fn f2_invites_table_exists_in_migration() {
    let pool = fresh_pool().await;
    let n: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='invites'",
    )
    .fetch_one(&pool)
    .await
    .expect("查 sqlite_master");
    assert_eq!(n, 1, "invites 表不存在 —— F1 的断言在测空气");
}

#[tokio::test]
async fn g1_key_queries_use_indexes() {
    let pool = fresh_pool().await;
    seed_two_users(&pool).await;

    let cases: Vec<(&str, &str)> =
        vec![
        ("用户会话列表（用户切换 <50ms 主路径）",
         "SELECT id,title,last_active_at FROM sessions WHERE user_id=? AND deleted_at IS NULL \
          ORDER BY last_active_at DESC LIMIT 20"),
        ("会话内消息倒序分页（深翻页）",
         "SELECT seq,content FROM messages WHERE user_id=? AND session_id=? \
          ORDER BY seq DESC LIMIT 50"),
        ("BM25 检索：按词取倒排",
         "SELECT doc_id,tf,field_len FROM wiki_index WHERE user_id=? AND term=?"),
        ("登录查用户",
         "SELECT id,password_hash FROM users WHERE username_norm=? AND deleted_at IS NULL"),
        ("refresh token 校验（每请求都跑）",
         "SELECT user_id,expires_at FROM sessions_auth WHERE token_hash=?"),
        ("派工收敛判定（AllSettled）",
         "SELECT COUNT(*) FROM task_dispatches WHERE user_id=? AND room_id=? \
          AND state IN ('PENDING','RUNNING','ASKING')"),
    ];

    for (name, sql) in cases {
        let rows = sqlx::query(&format!("EXPLAIN QUERY PLAN {sql}"))
            .bind(id(1))
            .bind(id(0x11))
            .fetch_all(&pool)
            .await
            .expect("取执行计划");
        let detail: Vec<String> = rows.iter().map(|r| r.get::<String, _>(3)).collect();
        let joined = detail.join(" | ");
        let full_scan = detail
            .iter()
            .any(|d| d.contains("SCAN") && !d.contains("USING"));
        assert!(
            !full_scan,
            "❌ [{name}] 走了全表扫描 → {joined}\n\
             忘了在查询里带 user_id？（见文档 §3 复合主键最左前缀）"
        );
        assert!(!joined.is_empty(), "❌ [{name}] 执行计划为空");
    }
}

#[tokio::test]
async fn g2_wiki_index_is_without_rowid() {
    let pool = fresh_pool().await;
    let sql: String = sqlx::query_scalar(
        "SELECT COALESCE(sql,'') FROM sqlite_master WHERE type='table' AND name='wiki_index'",
    )
    .fetch_one(&pool)
    .await
    .expect("查表定义");
    assert!(
        sql.to_uppercase().contains("WITHOUT ROWID"),
        "wiki_index 不是 WITHOUT ROWID —— 倒排表多了一层无用的 B-tree"
    );
}

#[tokio::test]
async fn g3_migration_matches_contract_table_list() {
    let pool = fresh_pool().await;
    let expected = [
        "experts",
        "invites",
        "mcp_servers",
        "messages",
        "plugins",
        "schema_version",
        "sessions",
        "sessions_auth",
        "skills",
        "task_dispatches",
        "team_members",
        "teams",
        "users",
        "wiki_index",
    ];
    let rows = sqlx::query(
        "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
    ).fetch_all(&pool).await.expect("查表清单");
    let actual: Vec<String> = rows.iter().map(|r| r.get::<String, _>(0)).collect();
    assert_eq!(
        actual, expected,
        "表清单与契约 §6 不符\n  期望: {expected:?}\n  实际: {actual:?}"
    );
}
