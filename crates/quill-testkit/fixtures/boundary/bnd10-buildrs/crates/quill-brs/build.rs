// 🚨 违规：构建脚本里生成含全局单例的代码
// 纯扫 src/ 的闸门看不到它；不扫 build.rs 的闸门也看不到它。

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let generated = r#"
        pub fn generated_singleton() -> &'static str {
            "Config::global()"
        }
    "#;
    let out = std::path::Path::new("src/generated.rs");
    std::fs::write(out, generated).expect("write generated");
}
