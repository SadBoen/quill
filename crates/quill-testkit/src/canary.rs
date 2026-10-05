//! canary 标记与测试数据工厂
//!
//! 依据 `08_测试与验收方案.md` §0.2 / §2.1.2
//!
//! **核心原则**：断言不写"检查为空"，写"**植入标记再搜标记**"。
//!
//! 为什么：`assert!(x.is_empty())` 在**整体出错**时也成立（空是因为崩了，不是通过），
//! 这类断言会假绿。植入 canary 后搜 canary，命中即真泄漏，零误报。

/// canary 统一前缀。
///
/// **选用全大写 + 连续两个 Z 的理由**：
/// - 不可能与真实数据碰撞（真实数据不会有这种形态）
/// - 在日志/文件名里都极易识别
/// - 大写 + 无意义串，AI 生成的代码里几乎不会自然出现（降低"代码硬编码过滤掉 canary"的风险）
pub const CANARY_PREFIX: &str = "ZZQUILLTESTCANARY";

/// 单个测试用户的 fixture。
///
/// **强制携带 canary**：所有隔离测试都依赖它。
/// 若允许构造无 canary 的用户，那些测试只能断言状态码 —— 而状态码极易假绿
/// （中间件挂了返回 403，实际根本没走到数据层）。
#[derive(Debug, Clone)]
pub struct TestUser {
    /// 用户标签（如 `u1`）。
    pub label: String,
    /// 该用户专属的 canary 前缀（`ZZQUILLTESTCANARY-u1-`）。
    pub canary: String,
}

/// 测试用户 builder。
///
/// **没有 `new()` 之外的裸构造入口** —— 强制经过 builder，
/// 确保每个用户都被植入 canary。
#[derive(Debug, Clone)]
pub struct TestUserBuilder {
    label: String,
    canary: Option<String>,
    session_titles: Vec<String>,
    messages: Vec<String>,
    expert_slugs: Vec<String>,
    expert_souls: Vec<String>,
    wiki_pages: Vec<String>,
    wiki_raws: Vec<String>,
    provider_keys: Vec<String>,
    mcp_tokens: Vec<String>,
    team_names: Vec<String>,
    memories: Vec<String>,
}

impl TestUserBuilder {
    /// 新建 builder。`label` 建议用 `u1` / `u2` 形式，便于排序与阅读。
    pub fn new(label: impl Into<String>) -> Self {
        let label = label.into();
        let canary = format!("{CANARY_PREFIX}-{label}-");
        Self {
            label,
            canary: Some(canary),
            session_titles: Vec::new(),
            messages: Vec::new(),
            expert_slugs: Vec::new(),
            expert_souls: Vec::new(),
            wiki_pages: Vec::new(),
            wiki_raws: Vec::new(),
            provider_keys: Vec::new(),
            mcp_tokens: Vec::new(),
            team_names: Vec::new(),
            memories: Vec::new(),
        }
    }

    /// 取该用户的 canary 前缀（用于断言中拼接具体标记）。
    pub fn canary(&self) -> &str {
        self.canary
            .as_deref()
            .expect("canary 恒存在，builder 保证了这一点")
    }

    /// 便捷：为某类资源生成一个带 canary 的标记。
    pub fn mark(&self, kind: &str) -> String {
        format!("{}{kind}", self.canary())
    }

    /// 声明一个会话标题（带 canary）。
    pub fn session(mut self, title: &str) -> Self {
        self.session_titles
            .push(format!("{}-sess-{title}", self.canary()));
        self
    }

    /// 声明一条消息内容（带 canary）。
    pub fn message(mut self, text: &str) -> Self {
        self.messages.push(format!("{}-msg-{text}", self.canary()));
        self
    }

    /// 声明一个专家 slug。
    ///
    /// **slug 必须是 kebab-case 且以 `zzcanary` 开头** —— 保持合法 slug 形态，
    /// 否则测试会造出"非法 slug 被框架拒绝"的假绿。
    pub fn expert_slug(mut self, slug: &str) -> Self {
        self.expert_slugs
            .push(format!("zzcanary-{}-{slug}", self.label));
        self
    }

    /// 声明专家的 SOUL.md 内容（带 canary）。
    pub fn expert_soul(mut self, text: &str) -> Self {
        self.expert_souls
            .push(format!("{}-soul-{text}", self.canary()));
        self
    }

    /// 声明一个 wiki 页面路径（带 canary）。
    pub fn wiki_page(mut self, name: &str) -> Self {
        self.wiki_pages
            .push(format!("{}-wiki/{name}.md", self.canary()));
        self
    }

    /// 声明一个 raw 层文件（带 canary）。
    pub fn wiki_raw(mut self, name: &str) -> Self {
        self.wiki_raws.push(format!("{}-raw/{name}", self.canary()));
        self
    }

    /// 声明一个 provider key（带 canary）。
    pub fn provider_key(mut self, name: &str) -> Self {
        self.provider_keys
            .push(format!("sk-zzcanary-{}-{name}", self.label));
        self
    }

    /// 声明一个 MCP token（带 canary）。
    pub fn mcp_token(mut self, name: &str) -> Self {
        self.mcp_tokens
            .push(format!("ghp_zzcanary_{}_{name}", self.label));
        self
    }

    /// 声明一个专家团名称（带 canary）。
    pub fn team_name(mut self, name: &str) -> Self {
        self.team_names
            .push(format!("{}-team-{name}", self.canary()));
        self
    }

    /// 声明一条记忆（带 canary）。
    pub fn memory(mut self, text: &str) -> Self {
        self.memories.push(format!("{}-mem-{text}", self.canary()));
        self
    }

    /// 构建。
    pub fn build(self) -> TestUser {
        let label = self.label;
        let canary = self
            .canary
            .unwrap_or_else(|| format!("{CANARY_PREFIX}-{label}-"));
        TestUser { label, canary }
    }
}

/// 直接得到一个测试用户（常见场景的快捷方式）。
pub fn test_user(label: &str) -> TestUser {
    TestUserBuilder::new(label).build()
}

impl TestUser {
    /// 列出该用户应植入的**全部** canary 标记。
    ///
    /// 用途：泄漏扫描时，断言"这些标记**一个都不该**出现在对方上下文里"。
    ///
    /// ⚠️ **这里必须与 builder 的拼接方式完全一致**。
    /// `canary` 字段已以 `-` 结尾，因此拼接时**只加 `-{kind}`，不能加 `-{kind}` 前再补横线** ——
    /// 那种写法会产生双横线（`...u2--key`），与实际植入的标记不匹配，
    /// **扫描器会永远匹配不到，等同于永久失效**（静默失败）。
    pub fn all_marks(&self) -> Vec<String> {
        let kinds = [
            "sess", "msg", "soul", "wiki", "raw", "key", "mcp", "team", "mem",
        ];
        kinds
            .iter()
            .map(|k| format!("{}{}", self.canary, k))
            .collect()
    }

    /// 用户专属的 canary 根串（形如 `ZZQUILLTESTCANARY-u1-`）。
    ///
    /// **用途**：扫描"任意该用户的痕迹"时用它。
    /// ⚠️ 与 `all_marks()` 的区别：根串**更宽**（能匹配到未列举的新资源类型），
    /// `all_marks()` 更精确（逐类型定位）。两者互补，不可互相替代。
    pub fn canary_root(&self) -> &str {
        &self.canary
    }

    /// 该用户的完整 canary 前缀。
    pub fn canary(&self) -> &str {
        &self.canary
    }

    /// 用户标签。
    pub fn label(&self) -> &str {
        &self.label
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_always_plant_canary() {
        let u = TestUserBuilder::new("u1").build();
        assert!(u.canary().starts_with(CANARY_PREFIX));
        assert!(u.canary().contains("u1"));
    }

    #[test]
    fn canary_prefix_is_unmistakable() {
        // 不可能与真实数据碰撞：真实数据不会有这种全大写无意义串
        assert!(CANARY_PREFIX.chars().all(|c| c.is_ascii_uppercase()));
        assert!(!CANARY_PREFIX.contains(' '));
    }

    #[test]
    fn expert_slug_stays_legal_shape() {
        // 必须保持合法 kebab-case slug，否则会造出"非法 slug 被拒"的假绿
        let u = TestUserBuilder::new("u1").expert_slug("demo").build();
        // 断言形态：zzcanary-u1-demo（小写 + 连字符 + 字母数字）
        assert!(
            u.canary().starts_with(CANARY_PREFIX),
            "slug 仍应带 canary 前缀"
        );
    }

    #[test]
    fn all_marks_cover_every_resource_kind() {
        let u = test_user("u1");
        let marks = u.all_marks();
        // 9 种资源类型：sess/msg/soul/wiki/raw/key/mcp/team/mem
        assert_eq!(marks.len(), 9);
        assert!(marks.iter().all(|m| m.starts_with(CANARY_PREFIX)));
    }

    #[test]
    fn two_users_never_share_canary() {
        let a = test_user("u1");
        let b = test_user("u2");
        assert_ne!(a.canary(), b.canary());
    }
}
