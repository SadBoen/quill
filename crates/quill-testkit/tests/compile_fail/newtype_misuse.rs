//! 🔴 **故意编译不过**的用例：把 `SessionId` 传给要 `UserId` 的函数。
//!
//! ⚠️ 本文件**不参与 `cargo test --workspace`** —— 它必然编译失败，
//! 若被 workspace 收录，`cargo test` 会立刻变红并掩盖真实错误。
//! 验证方式：`scripts/test-newtype-guard.sh` 编译它并断言**失败原因**。
//!
//! ⚠️ 若这些代码意外编译通过，说明身份类型退化成了 `type X = String` 别名，
//! 编译期隔离失效（这正是本文件要防的事）。

use quill_adapters::{ExpertId, MemberId, SessionId, UserId};

/// 正常签名：只收 `UserId`。
fn takes_user(_u: UserId) {}

fn main() {
    // 用例 ①：把 SessionId 当 UserId 传 —— 必须报 mismatched types
    let session = SessionId::from_bytes([2; 16]);
    takes_user(session);

    // 用例 ②：把 MemberId 当 UserId 传
    let member = MemberId::parse("cost-analyst-1").expect("合法");
    takes_user(member);

    // 用例 ③：把 ExpertId 当 UserId 传
    let expert = ExpertId::parse("cost-analyst").expect("合法");
    takes_user(expert);
}
