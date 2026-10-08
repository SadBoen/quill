//! 资料库的**模型侧接线**（queue Q057）。
//!
//! `quill-wiki` 的 `ingest` / `query` 早就写完了，是**完整的泛型实现**，只依赖
//! `quill_adapters::KnowledgeBackend`（三个方法）。全仓唯一的实现一直是测试里的
//! `FakeBackend`，所以这两条路在服务端一直是 501。这里补上真实现：拿**这一轮配置的
//! provider** 调模型 —— 与 `chat_compaction::ProviderSummarizer`（Q018）同一个模式：
//! 纯逻辑留在内层 crate，模型调用由壳注入。
//!
//! ## 模型输出什么形状（上游依据）
//!
//! 资料库不是 goose 的东西，是用户自己的 **xu-wiki**。它的架构是「CLI 只做确定性的活、
//! **CLI 不调 LLM**，由 agent 决定内容」（`design-docs/06-query.md` 的 `[PRIN-QRY-3]`；
//! `design-docs/05-ingest.md` 的 `[PRIN-ING-1]`「commit 是唯一写盘入口」）。
//! quill 里那个「agent」就是服务端自己，于是形状是：
//! **模型产出页面正文 / 答案 → 壳校验并写盘 → `quill-wiki` 重建索引、写日志**。
//!
//! 为了让结果可判定，这里要求模型回 **JSON** —— 这是 **quill 自己定的壳内协议**
//! （xu-wiki 没有这个 JSON，因为它让 agent 直接调 CLI）。协议写在提示词里，并在这里解析；
//! 解析失败、页面不合法都**报错**，不静默降级成「这次摄入什么都没做」。
//!
//! ## 读写分工
//!
//! `plan_ingest` 拿到的 `IngestContext` 里没有存储句柄，所以本模块按 `user` 自己开一个
//! `WikiStore` 写页；`quill-wiki::ingest` 随后会**把每一页读回来校验**再重建索引与日志
//! —— 也就是说「模型说写了」不算数，读得回来才算。

use std::path::PathBuf;

use quill_adapters::{
    AdapterError, IndexReceipt, IngestContext, KnowledgeBackend, KnowledgePage, LintContext,
    QueryAnswer, QueryContext, UserId,
};
use quill_core::llm::{build_request, LlmConfig};
use quill_provider::{Message, SharedProvider};
use quill_wiki::{page_from_wire, WikiStore};

/// 一次摄入最多接受几页。模型偶尔会把一页拆成十几页；那多半是它没读懂要求，
/// 与其写进去再让人一页页删，不如直接报错。
pub const MAX_PAGES_PER_INGEST: usize = 8;

/// 源文送模型的上限（字符）。超了就**截断并把这件事写进回执**，不静默丢内容。
pub const MAX_SOURCE_CHARS: usize = 60_000;

/// 索引与最近日志各自的上限（字符）——它们只是给模型的背景，不必全文送。
const MAX_CONTEXT_CHARS: usize = 20_000;

pub struct ProviderKnowledge {
    provider: SharedProvider,
    config: LlmConfig,
    wiki_dir: PathBuf,
}

impl std::fmt::Debug for ProviderKnowledge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderKnowledge")
            .field("model", &self.config.model)
            .finish_non_exhaustive()
    }
}

impl ProviderKnowledge {
    pub fn new(provider: SharedProvider, config: LlmConfig, wiki_dir: PathBuf) -> Self {
        Self {
            provider,
            config,
            wiki_dir,
        }
    }

    /// 调一次模型，把正文取回来。错误统一归到 `Provider`（**不猜**上游的错误种类）。
    fn complete(
        &self,
        system: String,
        user: String,
    ) -> impl std::future::Future<Output = Result<String, AdapterError>> + Send {
        let provider = self.provider.clone();
        let config = self.config.clone();
        async move {
            let request =
                build_request(&config, vec![Message::system(system), Message::user(user)]);
            match provider.chat(&request).await {
                Ok(reply) => Ok(reply.answer().to_string()),
                Err(e) => Err(AdapterError::Provider(format!(
                    "模型调用失败：{}。下一步：检查默认供应商的配置与凭据（用 `quill doctor` 诊断）。",
                    e.message()
                ))),
            }
        }
    }
}

/// 资料库页面的写法约束。**与 `quill-wiki` 的解析规则一一对应** ——
/// 少写一条，模型就会产出被 `page_from_wire` 拒掉的页面。
fn page_rules() -> String {
    "资料库的页面约定：\n\
     - 每一页是一个 markdown 文件；**第一行必须是 `---`**，frontmatter 必须闭合；\n\
     - frontmatter 至少要有 `title`、`type`、`created`、`updated`；\n\
     - `type` 取 concept / entity / comparison / overview / synthesis / summary 之一；\n\
     - `path` 是资料库内的相对路径（如 `concepts/入门.md`），**不许**用 `..` 或绝对路径；\n\
     - 正文用中文，结构清晰（标题 + 段落或列表）。\n"
        .to_string()
}

fn with_schema(mut s: String, schema: Option<&str>) -> String {
    if let Some(schema) = schema.filter(|t| !t.trim().is_empty()) {
        s.push_str("\n资料库自己的 schema（与上面冲突时**以它为准**）：\n");
        s.push_str(schema);
        s.push('\n');
    }
    s
}

fn clip(text: &str, max: usize) -> (String, bool) {
    if text.chars().count() <= max {
        return (text.to_string(), false);
    }
    (text.chars().take(max).collect(), true)
}

/// 从模型回复里取出**第一个括号平衡的 JSON 对象**。
///
/// 模型常把 JSON 包在 ```json 围栏里或前后带解释，所以不能直接 `from_str` 整个回复。
/// 逐个字节扫、跟踪字符串状态：字符串里的 `{` `}` 不算括号（否则 `"正文里有 }"` 会把它带偏）。
fn first_json_object(text: &str) -> Option<&str> {
    let start = text.find('{')?;
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (i, b) in text.bytes().enumerate().skip(start) {
        if in_string {
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'"' {
                in_string = false;
            }
            continue;
        }
        match b {
            b'"' => in_string = true,
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    // `start` 与 `i` 都是 ASCII 的 `{`/`}`，必然落在字符边界上。
                    return text.get(start..=i);
                }
            }
            _ => {}
        }
    }
    None
}

fn bad_output(what: &str, raw: &str) -> AdapterError {
    let (head, _) = clip(raw, 300);
    AdapterError::Internal(format!(
        "模型没有按约定回 JSON（{what}）。下一步：确认模型能按提示词输出 JSON；\
         原始回复前 300 字：{head}"
    ))
}

fn parse_pages(parsed: &serde_json::Value) -> Result<&Vec<serde_json::Value>, AdapterError> {
    let pages = parsed
        .get("pages")
        .and_then(|v| v.as_array())
        .ok_or_else(|| {
            AdapterError::Internal(
                "模型回的 JSON 里没有 `pages` 数组。下一步：提示词要求 \
                 {\"pages\":[{\"path\":…,\"content\":…}],\"summary\":…}。"
                    .to_string(),
            )
        })?;
    if pages.is_empty() {
        return Err(AdapterError::Internal(
            "模型回了空的 `pages`：这次摄入没有任何页面可写。\
             下一步：确认源文里确实有可成页的内容，或换一个模型重试。"
                .to_string(),
        ));
    }
    if pages.len() > MAX_PAGES_PER_INGEST {
        return Err(AdapterError::Internal(format!(
            "模型一次回了 {} 页，超过上限 {MAX_PAGES_PER_INGEST}。\
             下一步：把源文拆小再摄入（一次一份材料）。",
            pages.len()
        )));
    }
    Ok(pages)
}

impl KnowledgeBackend for ProviderKnowledge {
    fn plan_ingest(
        &self,
        user: UserId,
        ctx: IngestContext,
    ) -> impl std::future::Future<Output = Result<IndexReceipt, AdapterError>> + Send {
        let mut system = with_schema(
            format!(
                "你在往 quill 的资料库（一个 markdown wiki）里写入页面。\n\n{}\n\
                 只输出一个 JSON 对象，**不要**任何解释文字：\n\
                 {{\"pages\":[{{\"path\":\"concepts/xxx.md\",\"content\":\"---\\ntitle: …\\n…\"}}],\
                 \"summary\":\"这次摄入做了什么（一句话）\"}}\n",
                page_rules()
            ),
            ctx.schema_text.as_deref(),
        );
        system.push_str(
            "\n**只依据源文写页面**：源文里没有的事实不要编。索引与最近日志只是背景，\
             用来避免和已有页面重复。\n",
        );

        let (source_text, truncated) = clip(&ctx.source.text, MAX_SOURCE_CHARS);
        let (index_text, _) = clip(&ctx.index_text, MAX_CONTEXT_CHARS);
        let (recent_log, _) = clip(&ctx.recent_log, MAX_CONTEXT_CHARS);

        let mut user_prompt = format!(
            "源文（资料库内路径 `{}`）：\n\n{source_text}\n",
            ctx.source.rel_path
        );
        if truncated {
            user_prompt.push_str(&format!(
                "\n⚠️ 源文超过 {MAX_SOURCE_CHARS} 字符，**上面只是前 {MAX_SOURCE_CHARS} 字符**。\
                 请在 summary 里说明这一点。\n"
            ));
        }
        if !index_text.trim().is_empty() {
            user_prompt.push_str(&format!("\n当前索引（避免重复建页）：\n{index_text}\n"));
        }
        if !recent_log.trim().is_empty() {
            user_prompt.push_str(&format!("\n最近的变更日志：\n{recent_log}\n"));
        }
        if !ctx.related_pages.is_empty() {
            user_prompt.push_str("\n相关页面（可参考它们的写法）：\n");
            for p in &ctx.related_pages {
                user_prompt.push_str(&format!("--- {} ---\n{}\n", p.rel_path, p.content));
            }
        }

        let store = WikiStore::new(self.wiki_dir.clone(), user);
        async move {
            let raw = self.complete(system, user_prompt).await?;
            let json =
                first_json_object(&raw).ok_or_else(|| bad_output("找不到 JSON 对象", &raw))?;
            let parsed: serde_json::Value = serde_json::from_str(json).map_err(|e| {
                AdapterError::Internal(format!(
                    "模型回的 JSON 解析不了：{e}。下一步：换一个更稳的模型，或把提示词里的形状再强调一遍。"
                ))
            })?;
            let pages = parse_pages(&parsed)?;

            let mut touched = Vec::new();
            for page in pages {
                let path = page.get("path").and_then(|v| v.as_str()).ok_or_else(|| {
                    AdapterError::Internal(
                        "`pages` 里有条目缺少 `path`（必须是资料库内相对路径）。".to_string(),
                    )
                })?;
                let content = page
                    .get("content")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        AdapterError::Internal(format!("页面 {path:?} 缺少 `content`。"))
                    })?;
                // **先校验再写**：写不进去的页面（缺 frontmatter、路径越界）在这里就挡住，
                // 不让它进 index.md —— 否则下次 ingest 会把它当既有页继续往上叠。
                let page = page_from_wire(&KnowledgePage {
                    rel_path: path.to_string(),
                    content: content.to_string(),
                })
                .map_err(|e| {
                    AdapterError::Internal(format!(
                        "模型产出的页面 {path:?} 不是合法页面：{e}。\
                         下一步：重跑一次；若反复出现，把 schema 写得更明确些（schema/AGENTS.md）。"
                    ))
                })?;
                store
                    .write_page(&page.path, content)
                    .map_err(|e| AdapterError::Storage(format!("写页面 {path:?} 失败：{e}")))?;
                touched.push(page.path.clone());
            }

            let summary = parsed
                .get("summary")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .trim()
                .to_string();
            let summary = if truncated {
                format!("{summary}（注意：源文超过 {MAX_SOURCE_CHARS} 字符，只送了前 {MAX_SOURCE_CHARS} 字符给模型）")
            } else {
                summary
            };
            Ok(IndexReceipt { touched, summary })
        }
    }

    fn answer_query(
        &self,
        _user: UserId,
        ctx: QueryContext,
    ) -> impl std::future::Future<Output = Result<QueryAnswer, AdapterError>> + Send {
        let mut system = with_schema(
            "你在回答关于 quill 资料库的问题。\n\n\
             **只依据下面给出的候选页面作答**：页面里没有的事实不要编。\
             一个候选页都用不上就直说资料库里没有相关内容，并把 citations 给成空数组。\n\n\
             只输出一个 JSON 对象，**不要**任何解释文字：\n\
             {\"answer\":\"答案（中文）\",\"citations\":[\"concepts/xxx.md\"]}\n\
             `citations` 必须是**你真正用到的候选页面路径**。\n"
                .to_string(),
            ctx.schema_text.as_deref(),
        );
        system
            .push_str("\n提示：找不到答案时，答案是「资料库里没有相关内容」，不是「我不知道」。\n");

        let mut user_prompt = format!("问题：{}\n", ctx.question);
        if ctx.pages.is_empty() {
            user_prompt.push_str("\n（没有任何候选页面。）\n");
        } else {
            user_prompt.push_str("\n候选页面：\n");
            for p in &ctx.pages {
                user_prompt.push_str(&format!("--- {} ---\n{}\n", p.rel_path, p.content));
            }
        }

        async move {
            let raw = self.complete(system, user_prompt).await?;
            let json =
                first_json_object(&raw).ok_or_else(|| bad_output("找不到 JSON 对象", &raw))?;
            let parsed: serde_json::Value = serde_json::from_str(json)
                .map_err(|e| AdapterError::Internal(format!("模型回的 JSON 解析不了：{e}。")))?;
            let answer = parsed
                .get("answer")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| {
                    AdapterError::Internal(
                        "模型没有给出 `answer`（空或缺失）。下一步：重跑一次。".to_string(),
                    )
                })?
                .to_string();
            let citations = parsed
                .get("citations")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            Ok(QueryAnswer {
                answer,
                citations,
                archival_candidate: None,
            })
        }
    }

    fn lint_semantics(
        &self,
        _user: UserId,
        _ctx: LintContext,
    ) -> impl std::future::Future<Output = Result<Option<Vec<String>>, AdapterError>> + Send {
        // 语义体检还没做（`quill-wiki::lint` 目前只有机械规则）。**如实回 None**，
        // 而不是让模型随便说两句充当体检结果 —— 那会让「体检通过」变成一句空话。
        std::future::ready(Ok(None))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_object_is_extracted_from_fences_and_prose() {
        assert_eq!(first_json_object(r#"{"a":1}"#), Some(r#"{"a":1}"#));
        assert_eq!(
            first_json_object("好的：\n```json\n{\"a\":1}\n```\n以上。"),
            Some(r#"{"a":1}"#)
        );
        // 嵌套对象要整段取回。
        assert_eq!(
            first_json_object(r#"前言 {"a":{"b":2}} 后语"#),
            Some(r#"{"a":{"b":2}}"#)
        );
    }

    #[test]
    fn a_brace_inside_a_string_does_not_close_the_object() {
        // 这一条是核心：正文里写 `}` 是常态（模板、JSON 例子），
        // 不跟踪字符串状态就会在那里提前收尾，切出一个解析不了的片段。
        assert_eq!(
            first_json_object(r#"{"content":"正文里有 } 和 { 两个符号"}"#),
            Some(r#"{"content":"正文里有 } 和 { 两个符号"}"#)
        );
        // 转义引号同样不能把它带偏。
        assert_eq!(
            first_json_object(r#"{"c":"带 \" 引号 }"}"#),
            Some(r#"{"c":"带 \" 引号 }"}"#)
        );
    }

    #[test]
    fn no_object_or_unterminated_object_is_none() {
        assert_eq!(first_json_object("模型什么 JSON 都没给"), None);
        assert_eq!(
            first_json_object(r#"{"a":1"#),
            None,
            "没闭合要给 None，不能硬切"
        );
    }

    #[test]
    fn a_page_rule_block_mentions_everything_page_from_wire_demands() {
        // 提示词与校验器必须同源：`page_from_wire` 要求第一行 `---` 且 frontmatter 闭合。
        let rules = page_rules();
        assert!(rules.contains("第一行必须是 `---`"));
        assert!(rules.contains("frontmatter 必须闭合"));
        assert!(rules.contains("相对路径"));
    }

    #[test]
    fn clip_reports_whether_it_truncated() {
        assert_eq!(clip("abc", 3), ("abc".to_string(), false));
        assert_eq!(clip("abcd", 3), ("abc".to_string(), true));
        // 按**字符**数而不是字节：中文不会被切成半个字。
        assert_eq!(clip("中文中文", 2), ("中文".to_string(), true));
    }
}
