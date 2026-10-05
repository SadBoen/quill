
use quill_adapters::{ExpertId, MemberId, SessionId, UserId};

fn takes_user(_u: UserId) {}

fn main() {

    let session = SessionId::from_bytes([2; 16]);
    takes_user(session);

    let member = MemberId::parse("cost-analyst-1").expect("合法");
    takes_user(member);

    let expert = ExpertId::parse("cost-analyst").expect("合法");
    takes_user(expert);
}
