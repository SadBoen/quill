//! `quill-cli` 的行为测试。
//!
//! # 测什么
//!
//! 只测**能判红**的东西。凡是"跑一下看起来对"的输出，本测试**不测**。
//!
//! 重点是三条**容易悄悄坏掉**的边界：
//! 1. `--as <拼错的名字>` 读操作**不得**静默建号（否则用户以为是隔离坏了）
//! 2. `rm` 不带 `--yes` **不得**真删
//! 3. 退出码三态：0 / 1 / 2 各自可达且互不混淆

use quill_cli::{cmd_doctor, cmd_experts, Opts};
use std::path::{Path, PathBuf};

fn ws(tag: &str) -> PathBuf {
    let base = std::env::temp_dir().join(format!(
        "quill-cli-test-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).expect("建测试工作区");
    base
}

fn opts(base: &Path, user: &str, rest: Vec<&str>) -> Opts {
    Opts {
        user: user.into(),
        db: base.join("quill.db").to_string_lossy().into(),
        root: base.to_string_lossy().into(),
        yes: false,
        rest: rest.into_iter().map(String::from).collect(),
    }
}

#[test]
fn 读一个不存在的用户必须报错_不得静默建号() {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("建运行时");
    rt.block_on(async {
        let base = ws("readonly");
        let o = opts(&base, "alice", vec!["ls"]);
        let out = cmd_experts::run(&o).await;

        assert_eq!(
            out.code, 1,
            "读不存在的用户应是业务失败(1)，实得：{}",
            out.code
        );
        assert!(
            out.message.contains("不存在"),
            "错误信息没说明用户不存在：{}",
            out.message
        );
        assert!(
            out.message.contains("打错"),
            "错误信息没提示「可能只是名字打错了」：{}",
            out.message
        );

        // ★ 反向断言：这次读**不得**建出用户行
        let pool = quill_store::configure_pool(&base.join("quill.db").to_string_lossy(), 1)
            .await
            .expect("开库");
        let n: i64 = sqlx::query_scalar("SELECT count(*) FROM users")
            .fetch_one(&pool)
            .await
            .expect("查用户数");
        assert_eq!(n, 0, "读操作建出了 {n} 个用户行 —— 拼错名字会静默建号");
    });
}

#[test]
fn rm_不带_yes_不得真删() {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("建运行时");
    rt.block_on(async {
        let base = ws("rmguard");
        let mut o_add = opts(&base, "alice", vec!["add", "zhangsan", "张三", "对账"]);
        let created = cmd_experts::run(&o_add).await;
        assert_eq!(created.code, 0, "建专家失败：{}", created.message);

        // 不带 --yes
        let o_rm = opts(&base, "alice", vec!["rm", "zhangsan"]);
        let out = cmd_experts::run(&o_rm).await;
        assert_eq!(out.code, 0, "提示性输出不该是失败：{}", out.message);
        assert!(
            out.message.contains("--yes"),
            "没告诉用户要加 --yes：{}",
            out.message
        );

        // ★ 反向断言：专家还在
        o_add.rest = vec!["ls".into()];
        let ls = cmd_experts::run(&o_add).await;
        assert!(
            ls.message.contains("zhangsan"),
            "没加 --yes 却真删了：{}",
            ls.message
        );
    });
}

#[test]
fn 加_yes_才真删() {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("建运行时");
    rt.block_on(async {
        let base = ws("rmyes");
        let mut o_add = opts(&base, "alice", vec!["add", "lisi", "李四", "招聘"]);
        assert_eq!(cmd_experts::run(&o_add).await.code, 0);

        let mut o_rm = opts(&base, "alice", vec!["rm", "lisi"]);
        o_rm.yes = true;
        let out = cmd_experts::run(&o_rm).await;
        assert_eq!(out.code, 0, "带 --yes 的删除失败：{}", out.message);

        o_add.rest = vec!["ls".into()];
        let ls = cmd_experts::run(&o_add).await;
        assert!(
            !ls.message.contains("lisi"),
            "带 --yes 却没删掉：{}",
            ls.message
        );
    });
}

#[test]
fn 跨用户隔离_a看不到b的专家() {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("建运行时");
    rt.block_on(async {
        let base = ws("isolate");
        let a_add = opts(&base, "alice", vec!["add", "zhangsan", "张三", "对账"]);
        assert_eq!(cmd_experts::run(&a_add).await.code, 0);
        let mut b_add = opts(&base, "bob", vec!["add", "wangwu", "王五", "Bob的"]);
        assert_eq!(cmd_experts::run(&b_add).await.code, 0);

        let a_ls = opts(&base, "alice", vec!["ls"]);
        let la = cmd_experts::run(&a_ls).await;
        assert!(
            la.message.contains("zhangsan"),
            "Alice 看不到自己的：{}",
            la.message
        );
        assert!(
            !la.message.contains("wangwu"),
            "★ Alice 的列表里出现了 Bob 的专家 —— 隔离失效：{}",
            la.message
        );

        b_add.rest = vec!["ls".into()];
        let lb = cmd_experts::run(&b_add).await;
        assert!(
            lb.message.contains("wangwu"),
            "Bob 看不到自己的：{}",
            lb.message
        );
        assert!(!lb.message.contains("zhangsan"), "★ Bob 看到了 Alice 的");
    });
}

#[test]
fn doctor_全绿时必须退出_0() {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("建运行时");
    rt.block_on(async {
        let base = ws("doctor");
        let o = opts(&base, "alice", vec![]);
        let out = cmd_doctor::run(&o).await;
        // 🔴 这条专治"输出全 ✓ 却报有问题"——第一版真出现过这个错
        assert_eq!(
            out.code, 0,
            "全新环境下 doctor 应全绿，实得 {}：\n{}",
            out.code, out.message
        );
        assert!(out.message.contains('✓'), "没有任何 ✓ 项");
        assert!(
            out.message.contains("✅ 全部正常"),
            "缺少全绿结论行：\n{}",
            out.message
        );
        // ⚠️ 判据必须看**行首标记**，不能用子串：
        //    报告末尾的图例里就写着「✗ 有问题 / ▲ 无法判定」，
        //    `contains("有问题")` 会命中图例 —— 我第一版就是这么误判的。
        for line in out.message.lines() {
            let t = line.trim_start();
            assert!(
                !(t.starts_with('✗') || t.starts_with('▲')),
                "全绿却出现了失败/未判定行：{line}"
            );
        }
    });
}

#[test]
fn doctor_第二次跑_不能因迁移非幂等而失败() {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("建运行时");
    rt.block_on(async {
        let base = ws("doctor2");
        let o = opts(&base, "alice", vec![]);
        assert_eq!(cmd_doctor::run(&o).await.code, 0, "第一次跑挂了");
        // ★ `run_migration` **不是幂等的**（第二次会撞 "table already exists"）。
        //   这条钉住 CLI 的"先探测再迁移"逻辑。
        let second = cmd_doctor::run(&o).await;
        assert_eq!(
            second.code, 0,
            "第二次 doctor 失败（迁移非幂等未被挡住）：\n{}",
            second.message
        );
    });
}

#[test]
fn 未知选项必须拒绝_不得默默用默认值() {
    let args: Vec<String> = vec!["--as".into(), "alice".into(), "--asa".into(), "bob".into()];
    let r = Opts::parse(&args);
    assert!(r.is_err(), "拼错的选项 --asa 竟被接受");
    let msg = r.unwrap_err().message;
    assert!(msg.contains("--asa"), "错误信息没点名是哪个选项：{msg}");
}

#[test]
fn 参数顺序不影响命令识别() {
    // 🔴 第一版只认 args[0] 是命令，于是 `quill --root X doctor` 报「不认识的命令 --root」
    let cases: Vec<Vec<String>> = vec![
        vec!["experts".into(), "ls".into()],
        vec![
            "--root".into(),
            "/tmp".into(),
            "experts".into(),
            "ls".into(),
        ],
    ];
    for args in &cases {
        let got = quill_cli::split_command(args).expect("split 不应失败");
        assert_eq!(got, Some("experts"), "从 {args:?} 里认错了命令");
    }
}
