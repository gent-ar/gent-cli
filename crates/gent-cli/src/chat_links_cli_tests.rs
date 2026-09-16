use super::CallerConversation;

#[test]
fn a_caller_is_one_opaque_conversation_identity() {
    let caller = CallerConversation("conversation-1".into());
    assert_eq!(caller.0, "conversation-1");
    assert_eq!(caller, CallerConversation("conversation-1".into()));
}
