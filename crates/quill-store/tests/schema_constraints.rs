//! schema 约束断言 —— 把 `04_数据模型设计.md` 的隔离防线变成可执行测试。
//!
//! 移植自 `quill/.scratch/verify.py`（41 项断言，Python 版为设计期权威）。
//! 架构 §4.3「隔离性设计」在此落地为 `cargo test` 主链路的一部分。
//!
//! # ⚠️ 本文件最重要的部分不是断言，是 `expect_constraint` 这个装置
//!
//! 移植前的 Python 版本**出现过 3 次假绿**（详见文档附录 B.2）：
//!
//! | # | Python 版的错 | 为什么假绿 |
//! |---|---|---|
//! | 1 | 约束检查未传参 | 报 `Incorrect number of bindings`（**参数错误**），被宽泛的 `except sqlite3.Error` 捕获并当成"约束生效 ✅" |
//! | 2 | 复用同一 id/code_hash | 前一条主键冲突失败 → 后续行**没写进库** → 唯一约束永远不触发 |
//! | 3 | `code_hash` 写错字节数 | 撞上另一条 CHECK，行根本没进去 |
//!
//! 修正已**固化进本文件的装置**，不依赖作者记得：
//!
//! - [`expect_constraint`] **只认约束类错误码**（`SQLITE_CONSTRAINT*` 前缀）。
//!   参数错误、语法错误、类型错误一律判为**装置失效**并 `panic!`，
//!   绝不会被误读成"约束生效"。
//! - [`assert_legal`] 强制要求**合法数据必须能执行**。
//!   若这步失败，整个测试 fail（**不是 skip**），从而挡住上述第 2、3 类假绿。
//! - 参数与语句写进**同一个闭包**，杜绝「语句和参数对不上」这种装置失效。
//!
//! **原则**（装置可信性断言）：
//! **测试要"在该绿时，真的是因为对的原因而绿"。**

use sqlx::error::{DatabaseError, ErrorKind};
use sqlx::{Row, SqlitePool};

// ═══════════════ 装置：让假绿无处可藏 ═══════════════

/// 约束错误的语义种类；`None` 表示**装置失效**（语法错、参数错、类型错等）。
///
/// # 为什么用 `ErrorKind` 而不是错误文本
///
/// 第一版判定用「`code()` 文本是否以 `SQLITE_CONSTRAINT` 开头」——
/// 结果**全红**：sqlx 的 `Display` 输出是 `(code: 275) CHECK constraint failed: ...`，
/// `code()` 走 `sqlite3_errstr` 对扩展码返回的是数字串而非符号名。
///
/// 这正好证明本装置在工作：它把「我以为约束生效」判成了「装置失效」。
///
/// `DatabaseError::kind()` 由 sqlx 直接把 SQLite 扩展码映射成语义枚举
/// （`SQLITE_CONSTRAINT_CHECK` → `ErrorKind::CheckViolation`），
/// **与版本、措辞、显示格式全部无关** —— 这才是该用的判据。
fn constraint_kind(e: &dyn DatabaseError) -> Option<ErrorKind> {
    match e.kind() {
        ErrorKind::CheckViolation
        | ErrorKind::UniqueViolation
        | ErrorKind::ForeignKeyViolation
        | ErrorKind::NotNullViolation => Some(e.kind()),
        _ => None,
    }
}

/// `expect_constraint` / `assert_legal` 的执行结果类型。
type ExecResult = Result<sqlx::sqlite::SqliteQueryResult, sqlx::Error>;

/// 断言一条语句**因约束而被拒绝**。
///
/// # Panics
///
/// 两种情况都是**装置失效**，必须 fail 而非 skip：
/// - 语句被**接受** → 约束没生效（schema 与设计不符）
/// - 语句因**非约束原因**失败 → 这条测试本身写错了
///
/// ⚠️ 第二种最危险：宽泛的错误捕获会把它误读成"约束生效"，制造假绿。
/// 这正是 Python 版踩过的坑（文档附录 B.2 第 1 类）。
///
/// ⚠️ **此处曾有 `#[track_caller]`，已删除**（rustc lint
/// `ungated_async_fn_track_caller` 判红）。它是**死代码**：
/// `#[track_caller]` 无法穿过 `.await` 传播调用方位置
/// （rust-lang/rust#110011，stable 尚未实现），
/// 写了也不会让 panic 指向调用方 —— 属于"看起来在改善诊断、实际毫无作用"。
/// 定位需求由 panic 文案里的 `[{case}]` 满足（调用点已强制传 case 名），
/// 因此删除不损失任何定位能力。**请勿再加回。**
async fn expect_constraint<F, Fut>(case: &str, f: F)
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = ExecResult>,
{
    match f().await {
        Ok(_) => panic!("❌ [{case}] 语句被【接受】了 —— 预期应因约束被拒绝"),
        Err(sqlx::Error::Database(ref e)) => match constraint_kind(&**e) {
            Some(_) => { /* ✅ 这才是「约束按预期生效」 */ }
            None => panic!(
                "🚨 [{case}] 语句因【非约束原因】失败 —— 这是装置失效，不是约束生效！\n\
                 若把它当成「约束生效」就是假绿（第 1 类假闸门）。\n错误: {e}"
            ),
        },
        Err(other) => panic!("🚨 [{case}] 语句因【非数据库错误】失败 —— 装置失效：{other}"),
    }
}

/// 断言一条语句**合法且必须能执行**。
///
/// # 为什么每个测试都要配它
///
/// 挡住 Python 版第 2、3 类假绿：若前一条用例污染了状态（id 冲突）、
/// 或数据本身违反别的 CHECK（`code_hash` 长度写错），
/// 后续所有「约束生效」的断言都失去意义 —— 因为**行根本没进库**。
/// 没有这条，「约束生效」可能只是在读一个空的表。
///
/// ⚠️ 同 `expect_constraint`：曾有的 `#[track_caller]` 已删除 —— async fn 上是死代码。
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

// ═══════════════ 测试夹具 ═══════════════

/// 迁移文件内容（编译期嵌入，保证测试与迁移**永不漂移**）。
const MIGRATION: &str = include_str!("../migrations/0001_init.sql");

/// 16 字节 uuid 占位（测试不需要真实时间有序性，只要长度 16 且互不相同）。
fn id(n: u8) -> Vec<u8> {
    let mut v = vec![0u8; 16];
    v[0] = n;
    v[15] = 0xA0; // 标记这是测试数据
    v
}

fn h32(n: u8) -> Vec<u8> {
    vec![n; 32]
}

const NOW: i64 = 1_700_000_000_000;

/// 显式的密码算法串。
///
/// ⚠️ 这里**必须显式给值**：`users.password_algo` 曾经是
/// `DEFAULT 'argon2id'`，而实现跑的是 PBKDF2 —— 漏写该列的行会静默存下一个
/// 验证端根本不认的算法名，症状是"用户永远登不进去且全程无报错"。
/// 该默认值已被删除，现在漏写 = 立即 INSERT 失败（见 `password_algo_has_no_lying_default`）。
/// 本串的格式由 quill-control 的 `PasswordHasher::algo_tag()` 产出。
const ALGO: &str = "pbkdf2-hmac-sha256$i=600000";

/// 建库 + 跑迁移。迁移后 `run_migration` 已自检 `foreign_keys=ON`。
async fn fresh_pool() -> SqlitePool {
    let pool = quill_store::in_memory().await.expect("建内存库");
    quill_store::run_migration(&pool, MIGRATION)
        .await
        .expect("跑迁移");
    pool
}

/// 种两个用户 A / B，各带一个 session。所有跨用户断言的公共前提。
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

/// 种 A 的 team（供 team_members / task_dispatches 断言用）。
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

// 复用的语句常量
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

// ═══════════════ A. 跨用户复合外键 ═══════════════

/// ★ `password_algo` **没有**默认值，漏写该列必须**立刻失败**。
///
/// # 钉住的是什么
///
/// 该列曾是 `NOT NULL DEFAULT 'argon2id'`，而实现的算法是 PBKDF2-HMAC-SHA256
/// （`crates/quill-control/src/password.rs`）。这个默认值是**会骗人的 schema**：
/// 任何漏写该列的 INSERT 都会静默存下 `argon2id`，而验证端只认 `pbkdf2-*` 前缀
/// —— 症状是「这个用户永远登不进去」，且**没有任何一行报错**。
/// 静默失败比硬失败贵得多，所以默认值得删除、改为硬失败。
///
/// # 为什么"没有默认值"需要一条测试
///
/// 删掉一个 `DEFAULT` 是极其容易被"顺手加回来"的改动（看起来像无害的宽容）。
/// 没有这条测试，未来谁加回默认值，这个 crate 全绿、直到某天有人登不进那个账号。
#[tokio::test]
async fn password_algo_has_no_lying_default() {
    let pool = fresh_pool().await;

    // 漏写 password_algo → 必须被拒（因为没有默认值可吃）
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

    // 反向：空串也必须被 CHECK 拒掉（"给了列但没给值"同样不能蒙混过关）
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

    // 装置可信性：显式给真算法串必须能进库，否则上面两条可能只是在测空气
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

/// ★ 核心隔离机制：复合外键让「忘了加 where user_id」在数据库层被物理阻断。
#[tokio::test]
async fn a1_cross_user_foreign_keys_all_blocked() {
    let pool = fresh_pool().await;
    seed_two_users(&pool).await;

    // ★ 装置可信性：先证明合法数据能写进去，否则后面的"拦截"可能只是在测空气
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

    // 1. A 往 B 的 session 写消息
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

    // 2. A 建 team 时把 B 的 session 当 leader
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

    // 3. A 的 team_members 指向 B 的 session
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

    // 4. A 的派工指向 B 的 member session
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

/// 跨用户删除应被级联约束正确处理（不是静默留下孤儿行）。
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

// ═══════════════ B. slug / 路径：目录穿越的物理阻断 ═══════════════

#[tokio::test]
async fn b1_expert_slug_rejects_traversal() {
    let pool = fresh_pool().await;
    seed_two_users(&pool).await;

    // ★ 先证明合法 slug 能写 —— 否则"拦截"可能只是 slug 规则写坏了
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
    // tool_policy_json 为 NULL 必须被拒（TP-1：必须显式声明）
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

// ═══════════════ C. 状态机不变量由数据库强制 ═══════════════

#[tokio::test]
async fn c1_dispatch_state_machine_invariants() {
    let pool = fresh_pool().await;
    seed_two_users(&pool).await;
    seed_team_a(&pool).await;

    // ★ 先证明合法派工能写进去
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

    // 终态必须有 settled_at
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
    // 非终态不能有 settled_at
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
    // 非法状态
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
    // 幂等键：同 (room, round, member) 不许二次派工
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

    // ★ 先证明合法 leader 成员能写
    assert_legal("C2 前置：leader 成员", || async {
        // ?1 user_id ?2 team_id ?3 expert_id ?4 role ?5 member_session_id(NULL)
        // ?6 state ?7 state_changed_at ?8 joined_at ?9 created_at ?10 updated_at
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
    // 第二个 leader 必须被拒
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
    // 有 session 却处于 IDLE（未派工不该有会话）
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

    // ★ 先证明合法上限值能写
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

// ═══════════════ D. 消息序号与幂等 ═══════════════

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
    // 不同 session 各自的 seq=1 必须互不干扰
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

// ═══════════════ E. 软删除语义 ═══════════════

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

    // ★ X 必须是【存活】用户（deleted_at=NULL）。
    //   唯一索引是部分索引 `WHERE deleted_at IS NULL` —— 若 X 也被软删，
    //   它根本不在索引里，恢复 A 成功是**正确行为**而非约束失效。
    //   （第一版就是这里写错了：把"约束没生效"当成了测试失败。）
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

    // 现在恢复 A：X 存活且占用 alice → 部分唯一索引必须拒绝
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

    // ★ 反向自检：把 X 软删后，A 必须能恢复 —— 证明索引不是"永远拒绝"
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

// ═══════════════ F. invites：v1 保留结构、零写路径 ═══════════════

/// 裁决（team-lead 2026-10-04）：`invites` v1 建表但不启用。
/// 本测试证明**结构完整可用**（v1.1 启用时不需改 schema）。
#[tokio::test]
async fn f1_invites_structure_ready_for_v11() {
    let pool = fresh_pool().await;
    seed_two_users(&pool).await;

    // ★ 合法行必须能写 —— 这正是 v1.1 启用时要做的事
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
    // code_hash 必须恰好 32 字节
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
    // 重复 code_hash
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
    // 签发人不存在
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
    // 出生即过期（这条约束是移植过程中补上的）
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
    // used_count > max_uses
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

/// ★ 装置可信性：迁移文件必须真的建出了 invites 表。
/// 没有这条，F1 的断言可能在测一张不存在的表 —— 装置失效。
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

// ═══════════════ G. 索引与结构守卫 ═══════════════

/// 索引不是"建了就万事大吉"—— 必须真的被查询计划用上。
/// 忘了加 `user_id` 的后果是全表扫描，表现为 warm 切换远超 50ms 预算。
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
        // EXPLAIN 只需把参数填成占位值即可（不执行）
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

/// `wiki_index` 必须是 `WITHOUT ROWID`（纯倒排表，省一层 B-tree）。
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

/// ★ 迁移文件必须建出契约 §6 规定的全部 14 张表。
/// 这是「schema 与契约不漂移」的守卫 —— 少一张表会让后续成员的代码无从编译。
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
