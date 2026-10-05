
use std::sync::Arc;

use sqlx::{Row, SqlitePool};

use quill_control::{
    AuthPolicy, AuthSession, Clock, ControlError, ControlPlane, ManualClock, Pbkdf2Params,
    RegistrationRequest, SecretSource, UserProfile, UserRole, UserStatus,
};

const MIGRATION: &str = include_str!("../../quill-store/migrations/0001_init.sql");

async fn fresh_pool() -> SqlitePool {
    let pool = quill_store::in_memory().await.expect("建内存库");
    quill_store::run_migration(&pool, MIGRATION)
        .await
        .expect("跑迁移");
    pool
}

#[derive(Debug, Default)]
struct SeqSource {
    counter: std::sync::atomic::AtomicU64,
}

impl SecretSource for SeqSource {
    fn fill(&self, out: &mut [u8]) {
        use std::sync::atomic::Ordering;
        for slot in out.iter_mut() {
            let n = self.counter.fetch_add(1, Ordering::SeqCst);
            *slot = (n & 0xff) as u8 ^ ((n >> 8) & 0xff) as u8;
        }
    }
}

struct Fixture {
    plane: ControlPlane,
    clock: Arc<ManualClock>,
}

impl Fixture {
    async fn new() -> Self {
        let pool = fresh_pool().await;
        let clock = Arc::new(ManualClock::default());
        let plane = ControlPlane::with_policy(
            pool,
            Arc::clone(&clock) as Arc<dyn quill_control::Clock>,
            Arc::new(SeqSource::default()) as Arc<dyn SecretSource>,

            Pbkdf2Params::for_tests(),
            AuthPolicy::for_tests(),
        );
        Self { plane, clock }
    }

    async fn owner(&self, name: &str) -> UserProfile {
        self.plane
            .create_first_owner(&RegistrationRequest::new(
                name,
                "测试 Owner",
                "owner-password-1234",
            ))
            .await
            .unwrap_or_else(|e| panic!("建 owner 失败：{e}"))
    }

    async fn member(&self, owner: &UserProfile, name: &str) -> UserProfile {
        self.plane
            .create_user(
                &owner.id,
                &RegistrationRequest::new(name, "测试 Member", "member-password-1234"),
            )
            .await
            .unwrap_or_else(|e| panic!("建 member 失败：{e}"))
    }
}

const PW_OWNER: &str = "owner-password-1234";
const PW_MEMBER: &str = "member-password-1234";

#[tokio::test]
async fn first_owner_is_created_and_second_attempt_is_refused() {
    let f = Fixture::new().await;

    let owner = f.owner("owner_a").await;
    assert_eq!(owner.role, UserRole::Owner, "第一个用户必须是 owner");
    assert_eq!(owner.status, UserStatus::Active);
    assert_eq!(owner.username_norm, "owner_a");
    assert!(owner.last_login_at_ms.is_none(), "刚建号不该有登录时间");

    let err = f
        .plane
        .create_first_owner(&RegistrationRequest::new(
            "owner_b",
            "第二个 Owner",
            "another-password-99",
        ))
        .await
        .unwrap_err();
    assert_eq!(err, ControlError::FirstOwnerExists);
}

#[tokio::test]
async fn first_owner_role_cannot_be_downgraded_by_the_caller() {

    let f = Fixture::new().await;
    let owner = f
        .plane
        .create_first_owner(
            &RegistrationRequest::new("owner_x", "X", "owner-password-1234")
                .with_role(UserRole::Member),
        )
        .await
        .expect("建首个 owner");
    assert_eq!(owner.role, UserRole::Owner, "调用方不能把首个 owner 降级");
}

#[tokio::test]
async fn username_is_normalised_and_uniqueness_ignores_case() {
    let f = Fixture::new().await;
    let owner = f.owner("owner_a").await;

    let m = f
        .plane
        .create_user(
            &owner.id,
            &RegistrationRequest::new("  ZhangWei  ", "张伟", PW_MEMBER),
        )
        .await
        .expect("建号");
    assert_eq!(m.username, "ZhangWei", "原始写法应去掉首尾空白后保留");
    assert_eq!(m.username_norm, "zhangwei", "规范化形态应为小写");

    let dup = f
        .plane
        .create_user(
            &owner.id,
            &RegistrationRequest::new("ZHANGWEI", "另一个", PW_MEMBER),
        )
        .await
        .unwrap_err();
    assert_eq!(
        dup,
        ControlError::UsernameTaken {
            username_norm: "zhangwei".to_string()
        }
    );
}

#[tokio::test]
async fn invalid_registrations_are_rejected_with_human_readable_reasons() {
    let f = Fixture::new().await;
    let owner = f.owner("owner_a").await;

    let cases: Vec<(RegistrationRequest, ControlError)> = vec![
        (
            RegistrationRequest::new("ab", "太短", PW_MEMBER),
            ControlError::UsernameInvalid {
                raw: "ab".into(),
                reason: "少于 3 个字符",
            },
        ),
        (
            RegistrationRequest::new("zhang wei", "有空格", PW_MEMBER),
            ControlError::UsernameInvalid {
                raw: "zhang wei".into(),
                reason: "含空格 —— 用户名不能有空格，请用下划线",
            },
        ),
        (
            RegistrationRequest::new("good_name", "", PW_MEMBER),
            ControlError::DisplayNameInvalid {
                raw: String::new(),
                reason: "不能为空",
            },
        ),
        (
            RegistrationRequest::new("good_name", "名字", "short"),
            ControlError::PasswordTooShort {
                len: 5,
                min: quill_control::MIN_PASSWORD_LEN,
            },
        ),
        (
            RegistrationRequest::new("zhangwei2024", "同名密码", "zhangwei2024"),
            ControlError::PasswordEqualsUsername,
        ),
    ];
    for (req, want) in cases {
        let got = f.plane.create_user(&owner.id, &req).await.unwrap_err();
        assert_eq!(got, want, "请求 {req:?} 的错误不对");

        assert!(got.to_string().contains("quill doctor"));
    }
}

#[tokio::test]
async fn member_cannot_create_other_users() {
    let f = Fixture::new().await;
    let owner = f.owner("owner_a").await;
    let m = f.member(&owner, "member_a").await;

    let err = f
        .plane
        .create_user(
            &m.id,
            &RegistrationRequest::new("sneaky", "偷偷建号", PW_MEMBER),
        )
        .await
        .unwrap_err();
    assert_eq!(
        err,
        ControlError::NotAnOwner {
            operation: "创建用户"
        }
    );

    let ok = f
        .plane
        .create_user(
            &owner.id,
            &RegistrationRequest::new("second_member", "正常建号", PW_MEMBER),
        )
        .await
        .expect("owner 建号应成功");
    assert_eq!(ok.role, UserRole::Member);
}

#[tokio::test]
async fn login_with_correct_credentials_issues_usable_token() {
    let f = Fixture::new().await;
    let owner = f.owner("owner_a").await;

    let s = f
        .plane
        .login("owner_a", PW_OWNER, Some("test-agent/1.0"), None)
        .await
        .expect("正确凭据应登录成功");
    assert_eq!(s.user_id, owner.id);
    assert_eq!(s.role, UserRole::Owner);
    assert_eq!(s.username_norm, "owner_a");
    assert_eq!(
        s.token.len(),
        quill_control::TOKEN_HEX_LEN,
        "令牌必须是 64 位 hex（schema CHECK 依赖它）"
    );
    assert_eq!(
        s.expires_at_ms,
        s.issued_at_ms + f.plane.policy().session_ttl_millis
    );

    let auth = f.plane.authenticate(&s.token).await.expect("令牌应可用");
    assert_eq!(auth.user_id, owner.id);
    assert_eq!(auth.profile.username_norm, "owner_a");
    assert_eq!(auth.session_id, s.session_id);

    let after = f
        .plane
        .get_user(&owner.id, &owner.id)
        .await
        .expect("读档案");
    assert_eq!(
        after.last_login_at_ms,
        Some(s.issued_at_ms),
        "登录成功应写 last_login_at"
    );
}

#[tokio::test]
async fn login_accepts_mixed_case_and_surrounding_spaces_in_username() {
    let f = Fixture::new().await;
    f.owner("owner_a").await;

    f.plane
        .login("owner_a", PW_OWNER, None, None)
        .await
        .expect("规范形态");

    f.plane
        .login("  Owner_A  ", PW_OWNER, None, None)
        .await
        .expect("大小写与空白应被规范化");
}

#[tokio::test]
async fn login_does_not_reveal_whether_username_exists() {
    let f = Fixture::new().await;
    f.owner("owner_a").await;

    let wrong_pw = f
        .plane
        .login("owner_a", "wrong-password-xx", None, None)
        .await;
    let no_such_user = f
        .plane
        .login("ghost_user", "wrong-password-xx", None, None)
        .await;
    let bad_shape = f.plane.login("ab", "wrong-password-xx", None, None).await;

    for r in [wrong_pw, no_such_user, bad_shape] {
        assert_eq!(
            r.unwrap_err(),
            ControlError::CredentialsRejected,
            "三种失败必须返回同一个错误，否则可枚举用户名"
        );
    }

    let texts = [
        f.plane.login("owner_a", "x", None, None).await.unwrap_err(),
        f.plane.login("ghost", "x", None, None).await.unwrap_err(),
    ]
    .map(|e| e.to_string());
    assert_eq!(texts[0], texts[1], "失败文案也必须一致");
}

#[tokio::test]
async fn login_failure_message_never_contains_the_password() {
    let f = Fixture::new().await;
    f.owner("owner_a").await;
    let secret = "my-very-secret-password";
    let err = f
        .plane
        .login("owner_a", secret, None, None)
        .await
        .unwrap_err();
    assert!(
        !err.to_string().contains(secret),
        "错误消息泄露了密码：{err}"
    );
}

#[tokio::test]
async fn repeated_failures_lock_the_account_and_correct_password_is_refused() {
    let f = Fixture::new().await;
    f.owner("owner_a").await;
    let max = f.plane.policy().max_failures;

    for i in 1..max {
        assert_eq!(
            f.plane
                .login("owner_a", "wrong-password-xx", None, None)
                .await
                .unwrap_err(),
            ControlError::CredentialsRejected,
            "第 {i} 次失败不该暴露锁定状态"
        );
    }

    assert_eq!(
        f.plane
            .login("owner_a", "wrong-password-xx", None, None)
            .await
            .unwrap_err(),
        ControlError::CredentialsRejected
    );

    let err = f
        .plane
        .login("owner_a", PW_OWNER, None, None)
        .await
        .unwrap_err();
    match err {
        ControlError::AccountLocked { until_ms, .. } => {
            assert_eq!(
                until_ms,
                f.clock.now_millis() + f.plane.policy().lock_millis
            );
        }
        other => panic!("应报 AccountLocked，实际 {other:?}"),
    }
}

#[tokio::test]
async fn lock_expires_after_the_configured_window() {
    let f = Fixture::new().await;
    f.owner("owner_a").await;
    for _ in 0..f.plane.policy().max_failures {
        let _ = f
            .plane
            .login("owner_a", "wrong-password-xx", None, None)
            .await;
    }
    assert!(f
        .plane
        .login("owner_a", PW_OWNER, None, None)
        .await
        .is_err());

    f.clock.advance(f.plane.policy().lock_millis + 1);
    f.plane
        .login("owner_a", PW_OWNER, None, None)
        .await
        .expect("锁定过期后应能登录");
}

#[tokio::test]
async fn successful_login_resets_the_failure_counter() {
    let f = Fixture::new().await;
    f.owner("owner_a").await;

    for _ in 0..(f.plane.policy().max_failures - 1) {
        let _ = f
            .plane
            .login("owner_a", "wrong-password-xx", None, None)
            .await;
    }

    f.plane
        .login("owner_a", PW_OWNER, None, None)
        .await
        .expect("应能登录");

    for i in 1..f.plane.policy().max_failures {
        assert_eq!(
            f.plane
                .login("owner_a", "wrong-password-xx", None, None)
                .await
                .unwrap_err(),
            ControlError::CredentialsRejected,
            "第 {i} 次失败就报锁定 → 失败计数未被成功登录清零"
        );
    }
}

#[tokio::test]
async fn disabled_account_cannot_login_even_with_correct_password() {
    let f = Fixture::new().await;
    let owner = f.owner("owner_a").await;
    let m = f.member(&owner, "member_a").await;

    f.plane
        .set_user_status(&owner.id, &m.id, UserStatus::Disabled)
        .await
        .expect("禁用应成功");

    let err = f
        .plane
        .login("member_a", PW_MEMBER, None, None)
        .await
        .unwrap_err();
    assert_eq!(
        err,
        ControlError::AccountDisabled {
            username_norm: "member_a".to_string()
        }
    );

    f.plane
        .set_user_status(&owner.id, &m.id, UserStatus::Active)
        .await
        .expect("启用应成功");
    f.plane
        .login("member_a", PW_MEMBER, None, None)
        .await
        .expect("启用后应能登录");
}

#[tokio::test]
async fn owner_cannot_disable_itself() {
    let f = Fixture::new().await;
    let owner = f.owner("owner_a").await;
    let err = f
        .plane
        .set_user_status(&owner.id, &owner.id, UserStatus::Disabled)
        .await
        .unwrap_err();
    assert_eq!(err, ControlError::SelfDisableForbidden);
}

#[tokio::test]
async fn authenticate_rejects_malformed_and_unknown_tokens() {
    let f = Fixture::new().await;
    f.owner("owner_a").await;

    for bad in ["", "short", &"A".repeat(64), &"z".repeat(64)] {
        assert_eq!(
            f.plane.authenticate(bad).await.unwrap_err(),
            ControlError::TokenMalformed,
            "格式非法的令牌 {bad:?} 必须报 TokenMalformed"
        );
    }

    let ghost = "0".repeat(64);
    assert_eq!(
        f.plane.authenticate(&ghost).await.unwrap_err(),
        ControlError::SessionUnknown
    );
}

#[tokio::test]
async fn session_expires_after_ttl() {
    let f = Fixture::new().await;
    f.owner("owner_a").await;
    let s: AuthSession = f
        .plane
        .login("owner_a", PW_OWNER, None, None)
        .await
        .expect("登录");

    f.clock.advance(f.plane.policy().session_ttl_millis - 1);
    f.plane.authenticate(&s.token).await.expect("到期前应有效");

    f.clock.advance(1);
    assert_eq!(
        f.plane.authenticate(&s.token).await.unwrap_err(),
        ControlError::SessionExpired {
            expired_at_ms: s.expires_at_ms
        }
    );
}

#[tokio::test]
async fn disabled_user_loses_access_immediately_without_a_sweep_job() {
    let f = Fixture::new().await;
    let owner = f.owner("owner_a").await;
    let m = f.member(&owner, "member_a").await;
    let s = f
        .plane
        .login("member_a", PW_MEMBER, None, None)
        .await
        .expect("登录");
    f.plane.authenticate(&s.token).await.expect("禁用前应有效");

    f.plane
        .set_user_status(&owner.id, &m.id, UserStatus::Disabled)
        .await
        .expect("禁用");

    assert_eq!(
        f.plane.authenticate(&s.token).await.unwrap_err(),
        ControlError::AccountDisabled {
            username_norm: String::new()
        }
    );
}

#[tokio::test]
async fn logout_revokes_the_session_and_is_idempotent() {
    let f = Fixture::new().await;
    f.owner("owner_a").await;
    let s = f
        .plane
        .login("owner_a", PW_OWNER, None, None)
        .await
        .expect("登录");

    let out = f.plane.logout(&s.token).await.expect("登出");
    assert_eq!(out.revoked, 1, "首次登出应真的撤销 1 个会话");

    match f.plane.authenticate(&s.token).await.unwrap_err() {
        ControlError::SessionRevoked { reason } => {
            assert!(reason.contains("登出"), "原因：{reason}")
        }
        other => panic!("应报 SessionRevoked，实际 {other:?}"),
    }

    let again = f.plane.logout(&s.token).await.expect("二次登出应成功");
    assert_eq!(again.revoked, 0, "二次登出不应重复撤销");

    assert_eq!(
        f.plane.logout("nope").await.unwrap_err(),
        ControlError::TokenMalformed
    );
}

#[tokio::test]
async fn logout_of_unknown_but_wellformed_token_is_a_noop() {
    let f = Fixture::new().await;
    let out = f.plane.logout(&"0".repeat(64)).await.expect("应成功");
    assert_eq!(out.revoked, 0);
}

#[tokio::test]
async fn refresh_rotates_the_token_and_keeps_the_family() {
    let f = Fixture::new().await;
    f.owner("owner_a").await;
    let s1 = f
        .plane
        .login("owner_a", PW_OWNER, None, None)
        .await
        .expect("登录");

    let s2 = f.plane.refresh(&s1.token).await.expect("轮换应成功");
    assert_ne!(s2.token, s1.token, "轮换必须换新令牌");
    assert_eq!(s2.user_id, s1.user_id);
    assert_eq!(
        s2.expires_at_ms,
        s2.issued_at_ms + f.plane.policy().refresh_ttl_millis
    );

    f.plane.authenticate(&s2.token).await.expect("新令牌应可用");
    match f.plane.authenticate(&s1.token).await.unwrap_err() {
        ControlError::SessionRevoked { .. } => {}
        other => panic!("旧令牌应报 SessionRevoked，实际 {other:?}"),
    }
}

#[tokio::test]
async fn replaying_a_rotated_token_revokes_the_whole_family() {
    let f = Fixture::new().await;
    f.owner("owner_a").await;
    let s1 = f
        .plane
        .login("owner_a", PW_OWNER, None, None)
        .await
        .expect("登录");
    let s2 = f.plane.refresh(&s1.token).await.expect("轮换");

    let err = f.plane.refresh(&s1.token).await.unwrap_err();
    assert!(
        matches!(err, ControlError::SessionReuseDetected { .. }),
        "应报重放检出，实际 {err:?}"
    );

    assert!(
        f.plane.authenticate(&s2.token).await.is_err(),
        "检出重放后，同族的新令牌也必须失效"
    );
}

#[tokio::test]
async fn refresh_chain_stays_valid_across_multiple_rotations() {
    let f = Fixture::new().await;
    f.owner("owner_a").await;
    let mut cur = f
        .plane
        .login("owner_a", PW_OWNER, None, None)
        .await
        .expect("登录");
    for i in 1..=3 {
        let next = f
            .plane
            .refresh(&cur.token)
            .await
            .unwrap_or_else(|e| panic!("第 {i} 次轮换失败：{e}"));
        assert_ne!(next.token, cur.token);
        cur = next;
    }
    f.plane
        .authenticate(&cur.token)
        .await
        .expect("最新令牌应可用");
}

#[tokio::test]
async fn refresh_of_expired_or_unknown_token_is_refused() {
    let f = Fixture::new().await;
    f.owner("owner_a").await;
    let s = f
        .plane
        .login("owner_a", PW_OWNER, None, None)
        .await
        .expect("登录");

    assert_eq!(
        f.plane.refresh(&"0".repeat(64)).await.unwrap_err(),
        ControlError::SessionUnknown
    );
    assert_eq!(
        f.plane.refresh("bad").await.unwrap_err(),
        ControlError::TokenMalformed
    );

    f.clock.advance(f.plane.policy().session_ttl_millis + 1);
    assert!(matches!(
        f.plane.refresh(&s.token).await.unwrap_err(),
        ControlError::SessionExpired { .. }
    ));
}

#[tokio::test]
async fn change_password_requires_the_old_one_and_kills_all_sessions() {
    let f = Fixture::new().await;
    let owner = f.owner("owner_a").await;
    let s1 = f
        .plane
        .login("owner_a", PW_OWNER, None, None)
        .await
        .expect("登录1");
    let s2 = f
        .plane
        .login("owner_a", PW_OWNER, None, None)
        .await
        .expect("登录2");

    assert_eq!(
        f.plane
            .change_password(&owner.id, "wrong-old-password", "brand-new-pass-1")
            .await
            .unwrap_err(),
        ControlError::CredentialsRejected
    );

    let revoked = f
        .plane
        .change_password(&owner.id, PW_OWNER, "brand-new-pass-1")
        .await
        .expect("改密应成功");
    assert_eq!(revoked, 2, "两个会话都应被撤销");

    for s in [&s1, &s2] {
        assert!(
            f.plane.authenticate(&s.token).await.is_err(),
            "改密后旧令牌必须失效"
        );
    }

    assert!(f
        .plane
        .login("owner_a", PW_OWNER, None, None)
        .await
        .is_err());
    f.plane
        .login("owner_a", "brand-new-pass-1", None, None)
        .await
        .expect("新密码应能登录");
}

#[tokio::test]
async fn change_password_rejects_a_weak_new_password() {
    let f = Fixture::new().await;
    let owner = f.owner("owner_a").await;
    assert!(matches!(
        f.plane
            .change_password(&owner.id, PW_OWNER, "short")
            .await
            .unwrap_err(),
        ControlError::PasswordTooShort { .. }
    ));

    f.plane
        .login("owner_a", PW_OWNER, None, None)
        .await
        .expect("旧密码应仍可用");
}

#[tokio::test]
async fn change_password_advances_the_credential_epoch() {

    let f = Fixture::new().await;
    let owner = f.owner("owner_a").await;
    let before = f
        .plane
        .get_user(&owner.id, &owner.id)
        .await
        .expect("读档案")
        .token_epoch;

    f.plane
        .change_password(&owner.id, PW_OWNER, "another-pass-99")
        .await
        .expect("改密");
    let after = f
        .plane
        .get_user(&owner.id, &owner.id)
        .await
        .expect("读档案")
        .token_epoch;
    assert_eq!(after, before + 1, "改密必须推进凭据代次");
}

#[tokio::test]
async fn member_cannot_read_another_member() {
    let f = Fixture::new().await;
    let owner = f.owner("owner_a").await;
    let a = f.member(&owner, "member_a").await;
    let b = f.member(&owner, "member_b").await;

    f.plane.get_user(&a.id, &a.id).await.expect("A 读自己");
    f.plane.get_user(&b.id, &b.id).await.expect("B 读自己");

    for (x, y) in [(&a, &b), (&b, &a)] {
        assert_eq!(
            f.plane.get_user(&x.id, &y.id).await.unwrap_err(),
            ControlError::NotAnOwner {
                operation: "查看他人资料"
            },
            "{:?} 不该读到 {:?}",
            x.username_norm,
            y.username_norm
        );
    }
}

#[tokio::test]
async fn member_cannot_list_users_or_revoke_others_sessions() {
    let f = Fixture::new().await;
    let owner = f.owner("owner_a").await;
    let a = f.member(&owner, "member_a").await;
    let b = f.member(&owner, "member_b").await;

    assert!(matches!(
        f.plane.list_users(&a.id).await.unwrap_err(),
        ControlError::NotAnOwner { .. }
    ));
    assert!(matches!(
        f.plane.revoke_all_sessions(&a.id, &b.id).await.unwrap_err(),
        ControlError::NotAnOwner { .. }
    ));

    f.plane
        .revoke_all_sessions(&a.id, &a.id)
        .await
        .expect("撤销自己的会话应允许");
}

#[tokio::test]
async fn owner_can_read_any_user_and_list_all() {
    let f = Fixture::new().await;
    let owner = f.owner("owner_a").await;
    let a = f.member(&owner, "member_a").await;
    let b = f.member(&owner, "member_b").await;

    f.plane
        .get_user(&owner.id, &a.id)
        .await
        .expect("owner 读 A");
    f.plane
        .get_user(&owner.id, &b.id)
        .await
        .expect("owner 读 B");

    let all = f.plane.list_users(&owner.id).await.expect("owner 列出全部");
    assert_eq!(all.len(), 3, "应有 1 个 owner + 2 个 member");
}

#[tokio::test]
async fn one_users_token_never_authenticates_as_another_user() {

    let f = Fixture::new().await;
    let owner = f.owner("owner_a").await;
    let a = f.member(&owner, "member_a").await;
    let b = f.member(&owner, "member_b").await;

    let sa = f
        .plane
        .login("member_a", PW_MEMBER, None, None)
        .await
        .expect("A 登录");
    let auth = f
        .plane
        .authenticate(&sa.token)
        .await
        .expect("A 的令牌应有效");
    assert_eq!(auth.user_id, a.id, "令牌必须解析成签发它的那个用户");
    assert_ne!(auth.user_id, b.id);

    assert_eq!(auth.profile.role, UserRole::Member);
    assert_eq!(auth.profile.username_norm, "member_a");
}

#[tokio::test]
async fn unknown_actor_id_is_rejected_rather_than_treated_as_owner() {
    let f = Fixture::new().await;
    f.owner("owner_a").await;
    let ghost = quill_control::UserId::from_bytes([0xAB; 16]);
    assert_eq!(
        f.plane.list_users(&ghost).await.unwrap_err(),
        ControlError::UserNotFound,
        "不存在的用户不得被当成 owner"
    );
}

#[tokio::test]
async fn invite_redeem_creates_a_member_and_consumes_one_use() {
    let f = Fixture::new().await;
    let owner = f.owner("owner_a").await;

    let inv = f
        .plane
        .create_invite(&owner.id, UserRole::Member, 1)
        .await
        .expect("签发邀请码");
    assert_eq!(inv.code.len(), quill_control::INVITE_CODE_HEX_LEN);
    assert_eq!(inv.max_uses, 1);

    let u = f
        .plane
        .redeem_invite(
            &inv.code,
            &RegistrationRequest::new("invitee", "受邀者", "invitee-pass-1"),
        )
        .await
        .expect("兑换应成功");
    assert_eq!(
        u.role,
        UserRole::Member,
        "邀请码指定 member 就必须是 member"
    );
    assert_eq!(u.username_norm, "invitee");

    let list = f.plane.list_invites(&owner.id).await.expect("列出邀请码");
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].used_count, 1);
    assert_eq!(list[0].created_by, owner.id);
}

#[tokio::test]
async fn exhausted_invite_is_refused_and_creates_nobody() {
    let f = Fixture::new().await;
    let owner = f.owner("owner_a").await;
    let inv = f
        .plane
        .create_invite(&owner.id, UserRole::Member, 1)
        .await
        .expect("签发");

    f.plane
        .redeem_invite(
            &inv.code,
            &RegistrationRequest::new("first", "第一个", "first-pass-11"),
        )
        .await
        .expect("首次兑换");

    let err = f
        .plane
        .redeem_invite(
            &inv.code,
            &RegistrationRequest::new("second", "第二个", "second-pass-2"),
        )
        .await
        .unwrap_err();
    assert_eq!(err, ControlError::InviteExhausted { max_uses: 1 });

    assert!(
        f.plane.get_user(&owner.id, &owner.id).await.is_ok(),
        "owner 仍在"
    );
    let all = f.plane.list_users(&owner.id).await.expect("列出");
    assert_eq!(all.len(), 2, "只应有 owner + 第一个受邀者");
}

#[tokio::test]
async fn expired_invite_is_refused() {
    let f = Fixture::new().await;
    let owner = f.owner("owner_a").await;
    let inv = f
        .plane
        .create_invite(&owner.id, UserRole::Member, 1)
        .await
        .expect("签发");

    f.clock.advance(f.plane.policy().invite_ttl_millis - 1);
    assert!(f
        .plane
        .redeem_invite(
            &inv.code,
            &RegistrationRequest::new("early", "早鸟", "early-pass-111")
        )
        .await
        .is_ok());

    let inv2 = f
        .plane
        .create_invite(&owner.id, UserRole::Member, 1)
        .await
        .expect("签发2");
    f.clock.advance(f.plane.policy().invite_ttl_millis + 1);
    assert_eq!(
        f.plane
            .redeem_invite(
                &inv2.code,
                &RegistrationRequest::new("late", "迟到", "late-pass-111")
            )
            .await
            .unwrap_err(),
        ControlError::InviteExpired {
            expired_at_ms: inv2.expires_at_ms
        }
    );
}

#[tokio::test]
async fn revoked_invite_is_refused() {
    let f = Fixture::new().await;
    let owner = f.owner("owner_a").await;
    let inv = f
        .plane
        .create_invite(&owner.id, UserRole::Member, 1)
        .await
        .expect("签发");

    assert!(f
        .plane
        .revoke_invite(&owner.id, &inv.id)
        .await
        .expect("撤销"));
    assert_eq!(
        f.plane
            .redeem_invite(
                &inv.code,
                &RegistrationRequest::new("latecomer", "来晚", "late-pass-99")
            )
            .await
            .unwrap_err(),
        ControlError::InviteRevoked
    );

    assert_eq!(
        f.plane.revoke_invite(&owner.id, &inv.id).await.unwrap_err(),
        ControlError::InviteUnknown
    );
}

#[tokio::test]
async fn unknown_and_malformed_invite_codes_are_refused() {
    let f = Fixture::new().await;
    f.owner("owner_a").await;
    let req = RegistrationRequest::new("whoever", "某人", "whoever-pass-1");

    assert_eq!(
        f.plane.redeem_invite("nope", &req).await.unwrap_err(),
        ControlError::InviteMalformed
    );
    assert_eq!(
        f.plane
            .redeem_invite(&"0".repeat(32), &req)
            .await
            .unwrap_err(),
        ControlError::InviteUnknown
    );
}

#[tokio::test]
async fn invite_role_decides_the_new_users_role_and_request_cannot_override_it() {
    let f = Fixture::new().await;
    let owner = f.owner("owner_a").await;
    let inv = f
        .plane
        .create_invite(&owner.id, UserRole::Owner, 1)
        .await
        .expect("签发 owner 邀请码");

    let u = f
        .plane
        .redeem_invite(
            &inv.code,
            &RegistrationRequest::new("second_owner", "二号 Owner", "second-pass-11")
                .with_role(UserRole::Member),
        )
        .await
        .expect("兑换");
    assert_eq!(u.role, UserRole::Owner, "角色必须由邀请码决定");
}

#[tokio::test]
async fn member_cannot_issue_or_list_invites() {
    let f = Fixture::new().await;
    let owner = f.owner("owner_a").await;
    let m = f.member(&owner, "member_a").await;

    assert!(matches!(
        f.plane
            .create_invite(&m.id, UserRole::Member, 1)
            .await
            .unwrap_err(),
        ControlError::NotAnOwner { .. }
    ));
    assert!(matches!(
        f.plane.list_invites(&m.id).await.unwrap_err(),
        ControlError::NotAnOwner { .. }
    ));
}

#[tokio::test]
async fn owner_cannot_revoke_another_owners_invite() {

    let f = Fixture::new().await;
    let a = f.owner("owner_a").await;
    let inv = f
        .plane
        .create_invite(&a.id, UserRole::Member, 1)
        .await
        .expect("A 签发");
    assert_eq!(
        f.plane
            .create_first_owner(&RegistrationRequest::new(
                "owner_b",
                "B",
                "owner-b-password-1"
            ))
            .await
            .unwrap_err(),
        ControlError::FirstOwnerExists,
        "create_first_owner 在已有 owner 时必须失败"
    );

    let inv_b = f
        .plane
        .create_invite(&a.id, UserRole::Owner, 1)
        .await
        .expect("签发 owner 邀请码");
    let owner_b = f
        .plane
        .redeem_invite(
            &inv_b.code,
            &RegistrationRequest::new("owner_b", "B", "owner-b-password-1"),
        )
        .await
        .expect("兑换出第二个 owner");

    assert_eq!(
        f.plane
            .revoke_invite(&owner_b.id, &inv.id)
            .await
            .unwrap_err(),
        ControlError::InviteUnknown,
        "owner 只能撤销自己签发的邀请码"
    );

    let inv_b2 = f
        .plane
        .create_invite(&owner_b.id, UserRole::Member, 1)
        .await
        .expect("B 签发");
    assert!(f
        .plane
        .revoke_invite(&owner_b.id, &inv_b2.id)
        .await
        .expect("B 撤自己的"));
}

#[tokio::test]
async fn invite_with_zero_or_huge_max_uses_is_refused() {
    let f = Fixture::new().await;
    let owner = f.owner("owner_a").await;
    for bad in [0, -1, quill_control::MAX_INVITE_USES + 1] {
        assert_eq!(
            f.plane
                .create_invite(&owner.id, UserRole::Member, bad)
                .await
                .unwrap_err(),
            ControlError::InviteUsesOutOfRange {
                got: bad,
                max: quill_control::MAX_INVITE_USES
            },
            "max_uses={bad} 应被拒"
        );
    }
}

#[tokio::test]
async fn multi_use_invite_allows_exactly_max_uses_redeems() {
    let f = Fixture::new().await;
    let owner = f.owner("owner_a").await;
    let inv = f
        .plane
        .create_invite(&owner.id, UserRole::Member, 2)
        .await
        .expect("签发");

    for i in 1..=2 {
        f.plane
            .redeem_invite(
                &inv.code,

                &RegistrationRequest::new(format!("user{i}"), "用户", format!("user-{i}-pass-1")),
            )
            .await
            .unwrap_or_else(|e| panic!("第 {i} 次兑换应成功：{e}"));
    }

    assert_eq!(
        f.plane
            .redeem_invite(
                &inv.code,
                &RegistrationRequest::new("user3", "用户", "user-3-pass-1")
            )
            .await
            .unwrap_err(),
        ControlError::InviteExhausted { max_uses: 2 }
    );
    let list = f.plane.list_invites(&owner.id).await.expect("列出");
    assert_eq!(list[0].used_count, 2, "计数必须精确到 2");
}

#[tokio::test]
async fn invite_redeem_with_duplicate_username_rolls_back_the_whole_transaction() {

    let f = Fixture::new().await;
    let owner = f.owner("owner_a").await;
    f.member(&owner, "taken_name").await;
    let inv = f
        .plane
        .create_invite(&owner.id, UserRole::Member, 1)
        .await
        .expect("签发");

    assert_eq!(
        f.plane
            .redeem_invite(
                &inv.code,
                &RegistrationRequest::new("taken_name", "重名", "other-pass-123")
            )
            .await
            .unwrap_err(),
        ControlError::UsernameTaken {
            username_norm: "taken_name".to_string()
        }
    );
    let list = f.plane.list_invites(&owner.id).await.expect("列出");
    assert_eq!(list[0].used_count, 0, "兑换失败时邀请码不得被记账");
}

#[tokio::test]
async fn plaintext_password_is_never_written_to_the_database() {

    let f = Fixture::new().await;
    let owner = f.owner("owner_a").await;
    f.plane
        .login("owner_a", PW_OWNER, None, None)
        .await
        .expect("登录");

    f.plane
        .create_invite(&owner.id, UserRole::Member, 1)
        .await
        .expect("建邀请码，使 invites 表非空");

    let dump: String = sqlx::query("SELECT sql FROM sqlite_master")
        .fetch_all(f.plane.pool())
        .await
        .expect("读 schema")
        .into_iter()
        .filter_map(|r| r.try_get::<String, _>("sql").ok())
        .collect::<Vec<_>>()
        .join("\n");

    let pw_hex: String = PW_OWNER.bytes().map(|b| format!("{b:02x}")).collect();
    for (table, cols) in [
        ("users", "id,username,username_norm,display_name,password_hash,password_salt,password_algo,role,status,created_at"),
        ("sessions_auth", "id,user_id,token_hash,family_id,issued_at,expires_at"),
        ("invites", "id,code_hash,created_by,role,created_at"),
    ] {

        let searchable: Vec<String> = cols
            .split(',')
            .map(|c| format!("COALESCE(hex({c}), '<null>')"))
            .collect();
        let sql = format!("SELECT {} FROM {table}", searchable.join(" || '␟' || "));
        let rows: Vec<String> = sqlx::query_scalar(&sql)
            .fetch_all(f.plane.pool())
            .await
            .unwrap_or_else(|e| panic!("读 {table} 失败：{e}"));

        assert!(!rows.is_empty(), "{table} 扫描了 0 行 —— 装置失效");

        let hexed: Vec<&str> = rows
            .iter()
            .flat_map(|x| x.split('␟'))
            .filter(|s| s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit()))
            .collect();
        assert!(
            !hexed.is_empty(),
            "{table} 扫描里没有任何 32 字节 hex 值 —— 哈希列很可能没被真正检查"
        );
        for r in &rows {
            assert!(!r.contains(&pw_hex), "{table} 的某列出现口令字节：{r}");
        }
    }
    assert!(dump.contains("CREATE TABLE users"), "schema 读取本身应成功");
}

#[tokio::test]
async fn registration_request_debug_hides_the_password() {
    let req = RegistrationRequest::new("someone", "某人", "super-secret-value");
    let shown = format!("{req:?}");
    assert!(
        !shown.contains("super-secret-value"),
        "Debug 泄露了密码：{shown}"
    );
    assert!(shown.contains("已隐藏"), "Debug 应显式标注已隐藏：{shown}");

    assert!(req.password == "super-secret-value");
}

#[tokio::test]
async fn control_plane_debug_does_not_dump_pool_or_entropy() {
    let f = Fixture::new().await;
    let shown = format!("{:?}", f.plane);
    assert!(
        !shown.contains("sqlite"),
        "Debug 泄露了连接池/数据库路径：{shown}"
    );
    assert!(
        !shown.contains("SeqSource"),
        "Debug 泄露了熵源类型：{shown}"
    );
}

#[tokio::test]
async fn user_profile_has_no_password_fields_at_all() {

    let f = Fixture::new().await;
    let owner = f.owner("owner_a").await;
    let shown = format!("{:?}", owner);
    for forbidden in ["password_hash", "password_salt", "password", "digest"] {
        assert!(
            !shown.to_lowercase().contains(forbidden),
            "UserProfile 的 Debug 出现 {forbidden}：{shown}"
        );
    }
}

#[tokio::test]
async fn production_params_are_not_weakened_by_the_test_shortcut() {

    let prod = Pbkdf2Params::production();
    assert!(
        prod.iterations >= 600_000,
        "生产迭代次数被削弱：{}",
        prod.iterations
    );
    assert!(prod.iterations > Pbkdf2Params::for_tests().iterations);

    let p = quill_control::AuthPolicy::production();
    assert!(p.max_failures >= 5, "生产失败阈值过低：{}", p.max_failures);
    assert!(
        p.session_ttl_millis <= 30 * 24 * 3600 * 1000,
        "会话有效期过长（长期令牌被盗用风险）"
    );
    assert!(p.lock_millis >= 60_000, "锁定窗口过短，挡不住爆破");
}
