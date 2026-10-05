
use crate::{store, Opts, Outcome};
use quill_adapters::UserId;
use quill_agent::{ExpertRegistry, ExpertRepository};
use quill_server::SqlxExpertRepository;
use std::sync::Arc;

pub async fn run(o: &Opts) -> Outcome {
    let sub = o.rest.first().map(String::as_str).unwrap_or("");
    match sub {
        "ls" | "list" | "" => list(o).await,
        "add" | "new" => add(o).await,
        "rm" | "remove" | "del" => rm(o).await,
        other => Outcome::fail(format!(
            "不认识的子命令 {other:?}。可用：ls / add / rm（`quill --help`）"
        )),
    }
}

async fn registry(
    o: &Opts,
    create_user: bool,
) -> Result<(ExpertRegistry<SqlxExpertRepository>, UserId), Outcome> {

    let pool = store::open_db(&o.db).await?;
    let (uid, created) = store::resolve_user(&pool, &o.user, create_user).await?;
    if created {
        eprintln!("（首次使用：已为你创建用户 {:?}）", o.user);
    }
    let bridge = quill_server::DbBridge::open(&o.db, 4)
        .map_err(|e| Outcome::undet(format!("连不上数据库（{e}）。这不是通过。")))?;
    Ok((SqlxExpertRepository::registry(Arc::new(bridge)), uid))
}

async fn list(o: &Opts) -> Outcome {
    let (reg, uid) = match registry(o, false).await {
        Ok(v) => v,
        Err(e) => return e,
    };
    let all = match reg.repo().list_owned(&uid) {
        Ok(e) => e,
        Err(e) => return Outcome::undet(format!("查专家失败（{e}）。这不是「没有专家」。")),
    };

    let experts: Vec<_> = all.iter().filter(|e| !e.is_deleted()).collect();

    if experts.is_empty() {
        return Outcome::ok(format!(
            "用户 {:?} 还没有任何专家。\n  造一个：quill experts add zhangsan 张三 \"负责对账\"",
            o.user
        ));
    }

    let mut out = format!("用户 {:?} 的专家（{} 个）：\n", o.user, experts.len());
    out.push_str(&format!(
        "  {} {} {} {}\n",
        pad("标识(slug)", 22),
        pad("名称", 12),
        pad("默认启用", 10),
        "说明"
    ));
    for e in &experts {
        out.push_str(&format!(
            "  {} {} {} {}\n",
            pad(e.id().as_str(), 22),
            pad(&truncate(e.display_name(), 12), 12),
            pad(if e.default_enabled() { "是" } else { "否" }, 10),
            truncate(e.description(), 40)
        ));
    }
    out.push_str(
        "\n  只列出了**你自己**的专家。别人的专家在这里看不到 —— \
         用 `--as bob` 换个人再跑一次，能看到隔离是生效的。",
    );
    Outcome::ok(out)
}

fn pad(s: &str, width: usize) -> String {
    let w: usize = s
        .chars()
        .map(|c| if c as u32 > 0x2E80 { 2 } else { 1 })
        .sum();
    format!("{s}{}", " ".repeat(width.saturating_sub(w)))
}

async fn add(o: &Opts) -> Outcome {
    let Some(slug) = o.rest.get(1).cloned() else {
        return Outcome::fail(String::from(
            "缺参数。用法：quill experts add <slug> <名称> [说明]\n\
             例：quill experts add zhangsan 张三 \"负责对账\"",
        ));
    };
    let name = o.rest.get(2).cloned().unwrap_or_else(|| slug.clone());
    let desc = o.rest.get(3).cloned().unwrap_or_default();

    let (reg, uid) = match registry(o, true).await {
        Ok(v) => v,
        Err(e) => return e,
    };
    let eid = match quill_adapters::ExpertId::parse(&slug) {
        Ok(i) => i,
        Err(e) => return Outcome::fail(format!("标识 {slug:?} 非法：{e}（要小写 kebab-case）")),
    };

    let expert = match quill_agent::Expert::user_authored(uid, eid, name.clone(), desc) {
        Ok(e) => e,
        Err(e) => return Outcome::fail(format!("专家参数不合法：{e}")),
    };

    if let Err(e) = reg.repo().put(&expert) {
        return Outcome::fail(format!("写入失败：{e}\n（专家**没有**被保存。）"));
    }

    Outcome::ok(format!(
        "已为用户 {:?} 创建专家 {}（{}）。\n  查它：quill experts ls --as {}",
        o.user, slug, name, o.user
    ))
}

async fn rm(o: &Opts) -> Outcome {
    let Some(slug) = o.rest.get(1).cloned() else {
        return Outcome::fail(String::from("缺参数。用法：quill experts rm <slug>"));
    };
    let (reg, uid) = match registry(o, false).await {
        Ok(v) => v,
        Err(e) => return e,
    };
    let eid = match quill_adapters::ExpertId::parse(&slug) {
        Ok(i) => i,
        Err(e) => return Outcome::fail(format!("标识 {slug:?} 非法：{e}（要小写 kebab-case）")),
    };

    let found = match reg.repo().get(&uid, &eid) {
        Ok(f) => f,
        Err(e) => return Outcome::undet(format!("查专家失败（{e}）。")),
    };
    let Some(mut expert) = found else {

        return Outcome::fail(format!(
            "专家 {slug:?} 不存在，或不属于你。\n\
             （本命令对「不存在」与「无权访问」返回同一句话，避免被用来枚举别人的专家。）"
        ));
    };

    if !o.yes {
        return Outcome::ok(format!(
            "即将删除专家 {slug:?}（{}）。\n\
             确认请加 --yes：quill experts rm {slug} --yes",
            expert.display_name()
        ));
    }

    if let Err(e) = expert.soft_delete(&uid) {
        return Outcome::fail(format!("软删失败：{e}\n**专家仍在**，别当成已删。"));
    }
    if let Err(e) = reg.repo().put(&expert) {
        return Outcome::undet(format!("落库失败（{e}）。**专家仍在**，别当成已删。"));
    }
    Outcome::ok(format!("已删除专家 {slug:?}（软删，数据仍在库里可恢复）。"))
}

fn truncate(s: &str, n: usize) -> String {
    let s = s.trim();
    if s.is_empty() {
        return "—".into();
    }
    if s.chars().count() <= n {
        return s.to_string();
    }
    format!(
        "{}…",
        s.chars().take(n.saturating_sub(1)).collect::<String>()
    )
}
