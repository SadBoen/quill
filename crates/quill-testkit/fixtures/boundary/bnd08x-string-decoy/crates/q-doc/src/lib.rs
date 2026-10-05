//! ⚠️ 已知盲区登记：见 README.md
//!
//! 下面两种形态**性质不同**，我实测过：
//!   ① api_docs()  —— 纯字符串，当前闸门**能**抓到（会报 G26-1）
//!   ② assembled() —— 运行时拼接，**抓不到**，且**设计上不可能抓到**
//!
//! 本 fixture 保留 ① 是为了证明"文档字符串不是盲区"，
//! 保留 ② 才是真正的设计性盲区。

/// 当前闸门能抓到这一条（对照组：证明闸门不是完全瞎的）
pub fn api_docs() -> &'static str { "Config::global()" }

/// 🔴 真正的盲区：源码里不存在 `Config::global()` 这个连续字符串
pub fn assembled() -> String { format!("Config::{}()", "global") }
