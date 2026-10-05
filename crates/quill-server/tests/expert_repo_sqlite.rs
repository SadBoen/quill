//! `ExpertRepository` 的**真库**集成测试（临时 SQLite 文件）。
//!
//! 覆盖任务要求的四项：
//! 1. 专家增删改查往返；
//! 2. 🔴 A 用户读不到 B 用户的专家（跨用户隔离，M3 判定项）；
//! 3. 软删后同名重建成功（证明 `put` 的 upsert 真的生效）；
//! 4. 额外：换一条桥接（模拟进程重启）后数据仍在 —— 这是「只有内存实现」时
//!    最致命的问题，本实现就是为了消灭它。
//!
//! # 为什么这些用例打的是真库而不是内存实现
//!
//! `quill-agent` 自己的 85 个测试跑的是内存仓库。内存仓库**复刻**了
//! 复合主键与唯一约束，但它没有 SQL、没有 CHECK、没有外键、也没有重启。
//! 本文件打的是 `quill-store` 的 schema 真表，因此能验到：
//! `ON CONFLICT` 的真实语义、`visibility` 的 CHECK、`deleted_at` 的部分索引。

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

    // 建
    let created = r
        .create_user_expert(a, new_expert("cost-analyst"))
        .expect("创建应成功");
    assert_eq!(created.id().as_str(), "cost-analyst");
    assert_eq!(created.owner(), a);
    assert!(!created.is_deleted());

    // 查（单条）
    let got = r.get_visible(&a, &e("cost-analyst")).expect("应可读");
    assert_eq!(got.display_name(), "cost-analyst 专家");
    assert_eq!(got.description(), "测试用描述");

    // 改（改名 / 改描述 / 关默认启用）
    r.rename(&a, &e("cost-analyst"), "成本分析师")
        .expect("改名应成功");
    r.redescribe(&a, &e("cost-analyst"), "新描述")
        .expect("改描述应成功");
    r.set_default_enabled(&a, &e("cost-analyst"), false)
        .expect("关默认启用应成功");

    // ⚠️ 这一步是本文件最有价值的断言：`default_enabled = false` 是**领域层
    // 无法从公开构造器直接造出**的状态，读回路径必须靠重放领域跃迁还原。
    // 若读回时把它丢成 true，这里就会红 —— 而内存实现不会暴露这个问题。
    let got = r.get_visible(&a, &e("cost-analyst")).expect("应可读");
    assert_eq!(got.display_name(), "成本分析师");
    assert_eq!(got.description(), "新描述");
    assert!(
        !got.default_enabled(),
        "读回必须保留 default_enabled=false（否则「关掉」在重启后自动复活）"
    );

    // 列（可见 = 未删 + 可见性允许）
    let list = r.list_visible(&a).expect("应可列");
    assert_eq!(list.len(), 1, "属主应看得见自己的 1 个专家：{list:?}");

    // 删（软删）
    assert!(r.delete(&a, &e("cost-analyst")).expect("首次删应成功"));
    assert!(
        r.list_visible(&a).expect("应可列").is_empty(),
        "已删专家对属主也不可见"
    );
    assert!(
        !r.delete(&a, &e("cost-analyst")).expect("二次删应幂等"),
        "二次删除必须返回 false（幂等），而不是报错"
    );
    // ⚠️ 行仍在库里（软删），但对所有人不可见。
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
    // 🔴 跨用户隔离（M3 判定项）。每一步都必须是「不存在」而不是「无权限」：
    //    报 Forbidden 等于确认「A 确实有这个专家」，那是跨用户枚举接口。
    let t = TestDb::new("expert-isolation");
    let r = registry(&t.bridge());
    let a = u(1);
    let b = u(2);
    r.create_user_expert(a, new_expert("secret-expert"))
        .expect("A 建专家应成功");

    // 读
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
    // ⚠️ 直接打存储层（绕过领域层的命名空间搜索）：B 用自己的 owner 去 get，
    //    必须拿不到 —— 因为 SQL 的 WHERE 带 owner_user_id。
    assert!(
        r.repo()
            .get(&b, &e("secret-expert"))
            .expect("B 读库应成功")
            .is_none(),
        "🔴 存储层必须按 owner 过滤，不能只按 id"
    );

    // 改 / 删
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
    // A 的行必须毫发无损
    let got = r.get_visible(&a, &e("secret-expert")).expect("A 仍可读");
    assert_eq!(got.display_name(), "secret-expert 专家");
}

#[test]
fn soft_delete_then_recreate_same_id_succeeds_and_keeps_one_row() {
    // 🔴 验 upsert 真的生效：软删后同名重建在复合主键上是**同一行**。
    //    若 `put` 被实现成纯 INSERT，这一步会在写库那一刻失败
    //    （而领域层已经返回了「创建成功」—— 层间不一致且静默）。
    let t = TestDb::new("expert-upsert");
    let r = registry(&t.bridge());
    let a = u(1);

    r.create_user_expert(a, new_expert("cost-analyst"))
        .expect("首次创建应成功");
    assert!(r.delete(&a, &e("cost-analyst")).expect("删除应成功"));
    // 已删的行对领域层「不存在」，因此同名重建是合法的
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

    // 反向用例：未删除时同名创建必须被领域层判红（名字没被释放）
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
    // 「只有内存实现」时最致命的问题：重启即失。
    // 这里换一条桥接（= 换一条连接池 + 工作线程 + 新的 Runtime），
    // 同一个库文件上的数据必须完好。
    let t = TestDb::new("expert-restart");
    {
        let r = registry(&t.bridge());
        r.create_user_expert(u(1), new_expert("cost-analyst"))
            .expect("创建应成功");
        r.rename(&u(1), &e("cost-analyst"), "成本分析师")
            .expect("改名应成功");
    }
    // 全新桥接：与上一条是**不同的线程与运行时**
    let fresh =
        Arc::new(quill_server::DbBridge::open(&t.path(), 2).expect("重新打开同一库文件应成功"));
    let r = registry(&fresh);
    let got = r
        .get_visible(&u(1), &e("cost-analyst"))
        .expect("重启后应仍可读");
    assert_eq!(got.display_name(), "成本分析师");
    assert_eq!(r.list_visible(&u(1)).expect("应可列").len(), 1);
    // 且隔离在重启后依然成立
    assert!(
        r.list_visible(&u(2)).expect("B 列专家应成功").is_empty(),
        "🔴 重启后隔离不能失效（说明 user_id 真的进了库，而不是只在内存里过滤）"
    );
}

#[test]
fn builtin_expert_is_shared_but_protected() {
    // 内置专家属主是全零 UserId（schema 的交叉 CHECK）。
    // 它对所有用户可见，但谁都改不了。
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
    // 两个端口方法的可见性口径必须**不同且都正确**：
    // list_owned = 「我名下的全部行（含已删）」，roster = 「我能用的」。
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
    // 名册口径必须与 list_visible 完全一致（否则「列表里选得到的却加不进团」）
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
