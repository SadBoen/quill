
use crate::{store, Opts, Outcome};
use quill_wiki::WikiStore;

pub async fn run(o: &Opts) -> Outcome {
    let sub = o.rest.first().map(String::as_str).unwrap_or("");
    match sub {
        "ls" | "list" => ls(o).await,
        "show" | "cat" => show(o).await,
        "" => Outcome::fail(String::from("缺子命令。可用：ls / show（`quill --help`）")),
        other => Outcome::fail(format!("不认识的子命令 {other:?}。可用：ls / show")),
    }
}

async fn store_of(o: &Opts) -> Result<(WikiStore, String), Outcome> {
    let root = store::data_root(&o.root)?;
    let base = root.join("wiki");
    std::fs::create_dir_all(&base)
        .map_err(|e| Outcome::undet(format!("建资料库目录 {} 失败：{e}", base.display())))?;
    let pool = store::open_db(&o.db).await?;

    let (uid, _) = store::resolve_user(&pool, &o.user, false).await?;
    let label = base.display().to_string();
    Ok((WikiStore::new(base, uid), label))
}

async fn ls(o: &Opts) -> Outcome {
    let (st, base) = match store_of(o).await {
        Ok(v) => v,
        Err(e) => return e,
    };
    let pages = match st.list_pages() {
        Ok(p) => p,
        Err(e) => return Outcome::undet(format!("列页面失败：{e}。这**不是**「资料库是空的」。")),
    };
    if pages.is_empty() {
        return Outcome::ok(format!(
            "资料库还是空的（{base}）。\n\
             三层结构：raw/ 放原件 · wiki/ 放 .md 页面 · schema/ 放约定。\n\
             往 wiki/ 里丢一个 .md，再 `quill wiki ls` 就能看到。"
        ));
    }
    let mut out = format!("资料库页面（{} 个）：\n", pages.len());
    for p in &pages {
        out.push_str(&format!("  {p}\n"));
    }
    out.push_str("\n  看内容：quill wiki show <页面名>");
    Outcome::ok(out)
}

async fn show(o: &Opts) -> Outcome {
    let Some(name) = o.rest.get(1).cloned() else {
        return Outcome::fail(String::from("缺参数。用法：quill wiki show <页面名>"));
    };
    let (st, _) = match store_of(o).await {
        Ok(v) => v,
        Err(e) => return e,
    };
    match st.read_page(&name) {
        Ok(page) => {
            let mut out = format!("标题: {}\n\n", page.title());
            out.push_str(&page.body);
            Outcome::ok(out)
        }
        Err(e) => Outcome::fail(format!("读不到页面 {name:?}：{e}")),
    }
}
