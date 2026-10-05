
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
