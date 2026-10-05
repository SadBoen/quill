//! 🚨 本 fixture 故意含一个失败的测试
pub fn add(a: i32, b: i32) -> i32 { a + b }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn intentionally_failing() {
        // 故意失败：证明"全量测试必须全绿"这条规则真的能拦住东西
        assert_eq!(add(2, 2), 5, "fixture 故意失败");
    }
}
