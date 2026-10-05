
pub mod placeholder {
    #[cfg(test)]
    mod tests {

        #[test]
        fn placeholder_crate_is_unimplemented_by_design() {
            assert_eq!(
                2 + 2,
                4,
                "占位 crate 的唯一用例：证明可编译可测试，不证明任何功能"
            );
        }
    }
}
