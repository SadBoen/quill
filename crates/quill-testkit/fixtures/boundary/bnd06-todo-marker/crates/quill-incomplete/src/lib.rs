//! 🚨 本 fixture 故意含残留标记
//!
//! 为什么这些标记要拦：无人 review 项目里，`todo!()` 意味着"这段路径从未跑过"。

/// 🚨 违规：未实现
pub fn not_yet() {
    todo!("quill-incomplete: 待实现")
}

/// 🚨 违规：未实现且会 panic
pub fn unsupported() {
    unimplemented!("quill-incomplete: 明确不支持")
}
