pub(super) fn cancelled_control_request_key(frame: &[u8]) -> Option<String> {
    let frame: serde_json::Value = serde_json::from_slice(frame).ok()?;
    if frame.get("method").and_then(serde_json::Value::as_str) != Some("serverRequest/resolved") {
        return None;
    }
    let request_id = frame
        .pointer("/params/requestId")
        .or_else(|| frame.pointer("/params/request_id"))
        .or_else(|| frame.get("requestId"))
        .or_else(|| frame.get("request_id"))?;
    match request_id {
        serde_json::Value::String(value) if !value.is_empty() => Some(value.clone()),
        serde_json::Value::Number(value) if value.as_u64().is_some() => Some(value.to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::cancelled_control_request_key;

    #[test]
    fn cancellation_key_accepts_native_request_id_shapes() {
        assert_eq!(
            cancelled_control_request_key(
                br#"{"method":"serverRequest/resolved","params":{"requestId":7}}"#,
            ),
            Some("7".into())
        );
        assert_eq!(
            cancelled_control_request_key(
                br#"{"method":"serverRequest/resolved","params":{"request_id":"request-1"}}"#,
            ),
            Some("request-1".into())
        );
        assert_eq!(
            cancelled_control_request_key(br#"{"method":"future"}"#),
            None
        );
    }
}
