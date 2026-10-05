pub const CANARY_PREFIX: &str = "ZZQUILLTESTCANARY";

#[derive(Debug, Clone)]
pub struct TestUser {
    pub label: String,

    pub canary: String,
}

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

    pub fn canary(&self) -> &str {
        self.canary
            .as_deref()
            .expect("canary 恒存在，builder 保证了这一点")
    }

    pub fn mark(&self, kind: &str) -> String {
        format!("{}{kind}", self.canary())
    }

    pub fn session(mut self, title: &str) -> Self {
        self.session_titles
            .push(format!("{}-sess-{title}", self.canary()));
        self
    }

    pub fn message(mut self, text: &str) -> Self {
        self.messages.push(format!("{}-msg-{text}", self.canary()));
        self
    }

    pub fn expert_slug(mut self, slug: &str) -> Self {
        self.expert_slugs
            .push(format!("zzcanary-{}-{slug}", self.label));
        self
    }

    pub fn expert_soul(mut self, text: &str) -> Self {
        self.expert_souls
            .push(format!("{}-soul-{text}", self.canary()));
        self
    }

    pub fn wiki_page(mut self, name: &str) -> Self {
        self.wiki_pages
            .push(format!("{}-wiki/{name}.md", self.canary()));
        self
    }

    pub fn wiki_raw(mut self, name: &str) -> Self {
        self.wiki_raws.push(format!("{}-raw/{name}", self.canary()));
        self
    }

    pub fn provider_key(mut self, name: &str) -> Self {
        self.provider_keys
            .push(format!("sk-zzcanary-{}-{name}", self.label));
        self
    }

    pub fn mcp_token(mut self, name: &str) -> Self {
        self.mcp_tokens
            .push(format!("ghp_zzcanary_{}_{name}", self.label));
        self
    }

    pub fn team_name(mut self, name: &str) -> Self {
        self.team_names
            .push(format!("{}-team-{name}", self.canary()));
        self
    }

    pub fn memory(mut self, text: &str) -> Self {
        self.memories.push(format!("{}-mem-{text}", self.canary()));
        self
    }

    pub fn build(self) -> TestUser {
        let label = self.label;
        let canary = self
            .canary
            .unwrap_or_else(|| format!("{CANARY_PREFIX}-{label}-"));
        TestUser { label, canary }
    }
}

pub fn test_user(label: &str) -> TestUser {
    TestUserBuilder::new(label).build()
}

impl TestUser {
    pub fn all_marks(&self) -> Vec<String> {
        let kinds = [
            "sess", "msg", "soul", "wiki", "raw", "key", "mcp", "team", "mem",
        ];
        kinds
            .iter()
            .map(|k| format!("{}{}", self.canary, k))
            .collect()
    }

    pub fn canary_root(&self) -> &str {
        &self.canary
    }

    pub fn canary(&self) -> &str {
        &self.canary
    }

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
        assert!(CANARY_PREFIX.chars().all(|c| c.is_ascii_uppercase()));
        assert!(!CANARY_PREFIX.contains(' '));
    }

    #[test]
    fn expert_slug_stays_legal_shape() {
        let u = TestUserBuilder::new("u1").expert_slug("demo").build();

        assert!(
            u.canary().starts_with(CANARY_PREFIX),
            "slug 仍应带 canary 前缀"
        );
    }

    #[test]
    fn all_marks_cover_every_resource_kind() {
        let u = test_user("u1");
        let marks = u.all_marks();

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
