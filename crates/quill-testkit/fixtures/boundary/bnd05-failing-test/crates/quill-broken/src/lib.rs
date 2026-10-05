
pub fn add(a: i32, b: i32) -> i32 { a + b }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn intentionally_failing() {

        assert_eq!(add(2, 2), 5, "fixture 故意失败");
    }
}
