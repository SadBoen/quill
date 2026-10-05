pub mod cmd_backup;
pub mod cmd_doctor;
pub mod cmd_experts;
pub mod cmd_wiki;
pub mod store;

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub code: u8,
    pub message: String,
}

impl Outcome {
    pub fn ok(msg: impl Into<String>) -> Self {
        Self {
            code: 0,
            message: msg.into(),
        }
    }

    pub fn fail(msg: impl Into<String>) -> Self {
        Self {
            code: 1,
            message: msg.into(),
        }
    }

    pub fn undet(msg: impl Into<String>) -> Self {
        Self {
            code: 2,
            message: msg.into(),
        }
    }
}

impl fmt::Display for Outcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

pub const USAGE: &str = r#"quill —— Quill 个人/团队 Agent 平台

用法：
  quill <命令> [子命令] [参数]

命令：
  doctor              体检：数据库、配置、备份、版本，一次说清哪里坏了、怎么修
  experts ls          列出我（当前用户）的专家
  experts add <slug> <名称> [说明]
                      新建一个专家
  experts rm <slug>   删除一个专家（需要 --yes 确认）
  wiki ls             列出资料库页面
  wiki show <页面名>  打印一个页面的内容
  backup <目标目录>    做一次一致性备份（VACUUM INTO）
  restore <备份目录>   从备份恢复（需要 --yes 确认）

全局参数（放在命令前后都行）：
  --as <用户名>       以哪个用户身份操作（默认 alice）
  --db <路径>         数据库文件（默认 <root>/quill.db）
  --root <路径>       数据根目录（默认 .quill）
  --yes               跳过确认（删除、恢复等破坏性操作）
  -h, --help          显示本帮助

退出码：
  0 成功    1 业务失败    2 无法判定（环境不对，不是通过）

例子：
  quill doctor
  quill experts add zhangsan 张三 "负责对账"
  quill experts ls --as bob
  quill backup ./backup-2026-10-05
"#;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Opts {
    pub user: String,
    pub db: String,
    pub root: String,
    pub yes: bool,

    pub rest: Vec<String>,
}

impl Opts {
    pub fn parse(args: &[String]) -> Result<Self, Outcome> {
        let mut o = Opts {
            user: "alice".into(),
            db: default_db(),
            root: default_root(),
            yes: false,
            rest: Vec::new(),
        };
        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "--as" | "--db" | "--root" => {
                    let key = args[i].clone();
                    let val = args.get(i + 1).cloned().ok_or_else(|| {
                        Outcome::fail(format!("{key} 后面缺参数。用 `quill --help` 看用法。"))
                    })?;
                    match key.as_str() {
                        "--as" => o.user = val,
                        "--db" => o.db = val,
                        _ => o.root = val,
                    }
                    i += 2;
                }
                "--yes" => {
                    o.yes = true;
                    i += 1;
                }
                a if a.starts_with("--") && a.len() > 2 => {
                    return Err(Outcome::fail(format!(
                        "不认识的选项 {a:?}。**已中止，未按默认值执行**——\
                         拼错的参数会让人以错误的身份/路径干活。\
                         用 `quill --help` 看可用选项。"
                    )));
                }
                _ => {
                    o.rest.push(args[i].clone());
                    i += 1;
                }
            }
        }
        Ok(o)
    }
}

fn default_root() -> String {
    ".quill".to_string()
}

fn default_db() -> String {
    format!("{}/quill.db", default_root())
}

pub fn split_command(args: &[String]) -> Result<Option<&str>, Outcome> {
    let mut cmd: Option<&str> = None;
    let mut rest: Vec<String> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if cmd.is_none() && !a.starts_with('-') {
            cmd = Some(a);
            i += 1;
            continue;
        }

        if cmd.is_none() && matches!(a.as_str(), "--as" | "--db" | "--root") {
            rest.push(a.clone());
            if let Some(v) = args.get(i + 1) {
                rest.push(v.clone());
            }
            i += 2;
            continue;
        }
        rest.push(a.clone());
        i += 1;
    }
    let _ = rest;
    Ok(cmd)
}

pub async fn dispatch(args: &[String]) -> Outcome {
    let Some(cmd) = split_command(args).unwrap_or(None) else {
        return Outcome::fail(String::from(
            "没给命令。用法：quill <命令>，例如 `quill doctor`（`quill --help` 看全部）",
        ));
    };

    let mut rest: Vec<String> = Vec::new();
    let mut seen_cmd = false;
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if !seen_cmd && a == cmd {
            seen_cmd = true;
            i += 1;
            continue;
        }
        rest.push(a.clone());
        i += 1;
    }

    let o = match Opts::parse(&rest) {
        Ok(o) => o,
        Err(e) => return e,
    };

    match cmd {
        "doctor" => cmd_doctor::run(&o).await,
        "experts" => cmd_experts::run(&o).await,
        "wiki" => cmd_wiki::run(&o).await,
        "backup" => cmd_backup::backup(&o).await,
        "restore" => cmd_backup::restore(&o).await,
        other => Outcome::fail(format!(
            "不认识的命令 {other:?}。可用：doctor / experts / wiki / backup / restore\
             （`quill --help` 有例子）"
        )),
    }
}
