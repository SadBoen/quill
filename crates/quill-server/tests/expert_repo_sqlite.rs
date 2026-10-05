mod common;

use std::sync::Arc;

use quill_adapters::{ExpertId, UserId};
use quill_agent::{AgentError, ExpertRegistry, ExpertRepository, NewExpert};
use quill_server::experts_repo::SqlxExpertRepository;

use common::{scalar_i64, TestDb};

fn u(n: u8) -> UserId {
    UserId::from_bytes([n; 16])
}

fn e(name: &str) -> ExpertId {
    ExpertId::parse(name).unwrap_or_else(|err| panic!("专家名 {name:?} 非法：{err}"))
}

fn new_expert(name: &str) -> NewExpert {
    NewExpert {
        id: e(name),
        display_name: format!("{name} 专家"),
        description: "测试用描述".to_string(),
        instructions: "你是测试用专家".to_string(),
        model: None,
        source_template: None,
    }
}

fn registry(db: &Arc<quill_server::DbBridge>) -> ExpertRegistry<SqlxExpertRepository> {
    SqlxExpertRepository::registry(Arc::clone(db))
}

#[test]
fn create_read_update_delete_roundtrip_on_a_real_database() {
    let t = TestDb::new("expert-roundtrip");
    let r = registry(&t.bridge());
    let a = u(1);

    let created = r
        .create_user_expert(a, new_expert("cost-analyst"))
        .expect("创建应成功");
    assert_eq!(created.id().as_str(), "cost-analyst");
    assert_eq!(created.owner(), a);
    assert!(!created.is_deleted());

    let got = r.get_visible(&a, &e("cost-analyst")).expect("应可读");
    assert_eq!(got.display_name(), "cost-analyst 专家");
    assert_eq!(got.description(), "测试用描述");

    r.rename(&a, &e("cost-analyst"), "成本分析师")
        .expect("改名应成功");
    r.redescribe(&a, &e("cost-analyst"), "新描述")
        .expect("改描述应成功");
    r.set_default_enabled(&a, &e("cost-analyst"), false)
        .expect("关默认启用应成功");

    let got = r.get_visible(&a, &e("cost-analyst")).expect("应可读");
    assert_eq!(got.display_name(), "成本分析师");
    assert_eq!(got.description(), "新描述");
    assert!(
        !got.default_enabled(),
        "读回必须保留 default_enabled=false（否则「关掉」在重启后自动复活）"
    );

    let list = r.list_visible(&a).expect("应可列");
    assert_eq!(list.len(), 1, "属主应看得见自己的 1 个专家：{list:?}");

    assert!(r.delete(&a, &e("cost-analyst")).expect("首次删应成功"));
    assert!(
        r.list_visible(&a).expect("应可列").is_empty(),
        "已删专家对属主也不可见"
    );
    assert!(
        !r.delete(&a, &e("cost-analyst")).expect("二次删应幂等"),
        "二次删除必须返回 false（幂等），而不是报错"
    );

    assert_eq!(
        scalar_i64(
            &t.bridge(),
            "SELECT COUNT(*) AS c FROM experts WHERE owner_user_id = x'01010101010101010101010101010101' AND id = 'cost-analyst' AND deleted_at IS NOT NULL"
        ),
        1,
        "软删必须是 deleted_at 置位，而不是物理删除"
    );
}

#[test]
fn other_user_can_neither_read_nor_modify_nor_delete_my_expert() {
    let t = TestDb::new("expert-isolation");
    let r = registry(&t.bridge());
    let a = u(1);
    let b = u(2);
    r.create_user_expert(a, new_expert("secret-expert"))
        .expect("A 建专家应成功");

    assert!(
        r.list_visible(&b).expect("B 列专家应成功").is_empty(),
        "🔴 B 不得在列表里看到 A 的自建专家"
    );
    assert!(
        r.repo().roster(&b).expect("B 取名册应成功").is_empty(),
        "🔴 B 的名册里不得出现 A 的私有专家"
    );
    assert_eq!(
        r.get_visible(&b, &e("secret-expert")).unwrap_err(),
        AgentError::ExpertNotFound {
            id: e("secret-expert")
        },
        "🔴 B 直接按 id 取必须得到 NotFound"
    );

    assert!(
        r.repo()
            .get(&b, &e("secret-expert"))
            .expect("B 读库应成功")
            .is_none(),
        "🔴 存储层必须按 owner 过滤，不能只按 id"
    );

    assert_eq!(
        r.rename(&b, &e("secret-expert"), "劫持").unwrap_err(),
        AgentError::ExpertNotFound {
            id: e("secret-expert")
        },
        "🔴 B 改名必须得到 NotFound（不是 NotModifiable）"
    );
    assert_eq!(
        r.delete(&b, &e("secret-expert")).unwrap_err(),
        AgentError::ExpertNotFound {
            id: e("secret-expert")
        },
        "🔴 B 删除必须得到 NotFound"
    );

    let got = r.get_visible(&a, &e("secret-expert")).expect("A 仍可读");
    assert_eq!(got.display_name(), "secret-expert 专家");
}

#[test]
fn soft_delete_then_recreate_same_id_succeeds_and_keeps_one_row() {
    let t = TestDb::new("expert-upsert");
    let r = registry(&t.bridge());
    let a = u(1);

    r.create_user_expert(a, new_expert("cost-analyst"))
        .expect("首次创建应成功");
    assert!(r.delete(&a, &e("cost-analyst")).expect("删除应成功"));

    let again = r
        .create_user_expert(a, new_expert("cost-analyst"))
        .expect("软删后同名重建必须成功（upsert 生效的证明）");
    assert!(!again.is_deleted(), "重建后的专家必须是存活状态");

    assert_eq!(
        scalar_i64(
            &t.bridge(),
            "SELECT COUNT(*) AS c FROM experts WHERE owner_user_id = x'01010101010101010101010101010101' AND id = 'cost-analyst'"
        ),
        1,
        "重建后仍必须只有 1 行（upsert 不是「删一行插一行」）"
    );
    let got = r.get_visible(&a, &e("cost-analyst")).expect("应可读");
    assert!(
        !got.is_deleted(),
        "重建后 deleted_at 必须被 upsert 置回 NULL，否则专家永远不可见"
    );

    let dup = r.create_user_expert(a, new_expert("cost-analyst"));
    assert_eq!(
        dup.unwrap_err(),
        AgentError::ExpertExists {
            id: e("cost-analyst")
        },
        "未删除的同名专家必须判重（upsert 不等于允许改名覆盖）"
    );
}

#[test]
fn data_survives_a_fresh_bridge_instance_simulating_process_restart() {
    let t = TestDb::new("expert-restart");
    {
        let r = registry(&t.bridge());
        r.create_user_expert(u(1), new_expert("cost-analyst"))
            .expect("创建应成功");
        r.rename(&u(1), &e("cost-analyst"), "成本分析师")
            .expect("改名应成功");
    }

    let fresh =
        Arc::new(quill_server::DbBridge::open(&t.path(), 2).expect("重新打开同一库文件应成功"));
    let r = registry(&fresh);
    let got = r
        .get_visible(&u(1), &e("cost-analyst"))
        .expect("重启后应仍可读");
    assert_eq!(got.display_name(), "成本分析师");
    assert_eq!(r.list_visible(&u(1)).expect("应可列").len(), 1);

    assert!(
        r.list_visible(&u(2)).expect("B 列专家应成功").is_empty(),
        "🔴 重启后隔离不能失效（说明 user_id 真的进了库，而不是只在内存里过滤）"
    );
}

#[test]
fn builtin_expert_is_shared_but_protected() {
    let t = TestDb::new("expert-builtin");
    let r = registry(&t.bridge());
    r.create_builtin_expert(new_expert("builtin-helper"))
        .expect("创建内置专家应成功");

    for viewer in [u(1), u(2), u(3)] {
        let list = r.list_visible(&viewer).expect("应可列");
        assert_eq!(list.len(), 1, "内置专家对用户 {viewer} 应可见");
        assert!(list[0].is_builtin());
        assert!(r
            .repo()
            .roster(&viewer)
            .expect("名册应可取")
            .contains(&e("builtin-helper")));
    }
    for viewer in [u(1), u(2)] {
        assert_eq!(
            r.rename(&viewer, &e("builtin-helper"), "劫持").unwrap_err(),
            AgentError::ExpertBuiltinProtected {
                id: e("builtin-helper")
            },
            "🔴 内置专家对任何用户都不可改"
        );
        assert_eq!(
            r.delete(&viewer, &e("builtin-helper")).unwrap_err(),
            AgentError::ExpertBuiltinProtected {
                id: e("builtin-helper")
            },
            "🔴 内置专家对任何用户都不可删"
        );
    }
}

#[test]
fn list_owned_includes_soft_deleted_rows_while_roster_does_not() {
    let t = TestDb::new("expert-visibility-scopes");
    let r = registry(&t.bridge());
    let a = u(1);
    r.create_user_expert(a, new_expert("alpha"))
        .expect("应创建");
    r.create_user_expert(a, new_expert("beta")).expect("应创建");
    r.delete(&a, &e("beta")).expect("删除应成功");

    let owned = r.repo().list_owned(&a).expect("应可列");
    assert_eq!(owned.len(), 2, "list_owned 必须含已软删行：{owned:?}");
    assert_eq!(
        owned.iter().filter(|e| e.is_deleted()).count(),
        1,
        "其中恰好 1 行已删"
    );
    let roster = r.repo().roster(&a).expect("名册应可取");
    assert_eq!(roster.len(), 1, "名册只含存活且可见的：{roster:?}");
    assert!(roster.contains(&e("alpha")));
    assert!(!roster.contains(&e("beta")));

    let visible: Vec<ExpertId> = r
        .list_visible(&a)
        .expect("应可列")
        .iter()
        .map(|x| x.id().clone())
        .collect();
    assert_eq!(
        roster.into_iter().collect::<Vec<ExpertId>>(),
        visible,
        "名册与列表口径必须逐条一致"
    );
}

/// 人格两列必须真的落库：非空 instructions、model 的 NULL 语义、以及 PATCH
/// 之后能读回新值（upsert 漏写这两列的话，接口返回 200 但下次读还是旧值）。
#[test]
fn persona_columns_persist_across_bridge_restart_including_null_model() {
    let t = TestDb::new("expert-persona-persist");
    {
        let r = registry(&t.bridge());
        let a = u(1);
        let mut n = new_expert("cost-analyst");
        n.instructions = "你是一名严谨的成本分析师。".into();
        n.model = Some("qwen3-max".into());
        r.create_user_expert(a, n).expect("创建应成功");

        let mut blank = new_expert("quiet-expert");
        blank.instructions = String::new();
        blank.model = None;
        r.create_user_expert(a, blank).expect("空人格必须能建");
    }

    let fresh =
        Arc::new(quill_server::DbBridge::open(&t.path(), 2).expect("重新打开同一库文件应成功"));
    let r = registry(&fresh);

    let got = r
        .get_visible(&u(1), &e("cost-analyst"))
        .expect("重启后应仍可读");
    assert_eq!(got.instructions(), "你是一名严谨的成本分析师。");
    assert_eq!(got.model(), Some("qwen3-max"));

    let quiet = r
        .get_visible(&u(1), &e("quiet-expert"))
        .expect("重启后应仍可读");
    assert_eq!(quiet.instructions(), "", "空人格必须原样保留成空串");
    assert_eq!(
        quiet.model(),
        None,
        "🔴 model 的 NULL（= 跟随实例默认模型）不能被读成空串"
    );

    r.set_instructions(&u(1), &e("quiet-expert"), "改过的人格")
        .expect("改人格应成功");
    r.set_model(&u(1), &e("cost-analyst"), None)
        .expect("清除偏好模型应成功");

    let updated = r
        .get_visible(&u(1), &e("quiet-expert"))
        .expect("应可读");
    assert_eq!(
        updated.instructions(),
        "改过的人格",
        "🔴 PATCH 的人格必须真的写进库（PUT_SQL 的 DO UPDATE 段漏了 instructions）"
    );
    let cleared = r
        .get_visible(&u(1), &e("cost-analyst"))
        .expect("应可读");
    assert_eq!(
        cleared.model(),
        None,
        "🔴 显式清除 model 必须落成 NULL，而不是空串"
    );
    assert_eq!(
        scalar_i64(
            &fresh,
            "SELECT COUNT(*) AS c FROM experts WHERE id = 'cost-analyst' AND model IS NULL"
        ),
        1,
        "库里必须真的是 NULL"
    );
}
