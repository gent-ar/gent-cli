//! ACP permission wire decoding. Only trusted host session/prompt binds session.
use super::{Error, Value};
pub struct Request {
    pub key: String,
    pub tool: String,
    pub args: Value,
    pub allow: Vec<String>,
}
fn bounded(v: &Value) -> Result<&str, Error> {
    v.as_str()
        .filter(|v| !v.is_empty() && v.len() <= 512)
        .ok_or(Error::Protocol)
}
fn id(v: &Value) -> Result<String, Error> {
    if let Some(text) = v.as_str() {
        return Ok(format!("acp:s:{}", bounded(&Value::String(text.into()))?));
    }
    if v.as_i64().is_some() || v.as_u64().is_some() {
        return Ok(format!("acp:n:{v}"));
    }
    Err(Error::Protocol)
}
pub fn trusted_session(frame: &Value) -> Result<Option<String>, Error> {
    if frame["jsonrpc"] != "2.0" || frame["method"] != "session/prompt" {
        return Ok(None);
    }
    Ok(Some(bounded(&frame["params"]["sessionId"])?.into()))
}
pub fn request(frame: &Value, session: Option<&str>) -> Result<Option<Request>, Error> {
    if frame["jsonrpc"] != "2.0" || frame["method"] != "session/request_permission" {
        return Ok(None);
    }
    let params = &frame["params"];
    let call = &params["toolCall"];
    let tool = bounded(&call["title"])?;
    if !tool.starts_with("mcp__") {
        return Ok(None);
    }
    if session != Some(bounded(&params["sessionId"])?) {
        return Err(Error::Protocol);
    }
    bounded(&call["toolCallId"])?;
    let args = call["rawInput"].clone();
    if !args.is_object() {
        return Err(Error::Protocol);
    }
    let options = params["options"]
        .as_array()
        .filter(|v| !v.is_empty() && v.len() <= 16)
        .ok_or(Error::Protocol)?;
    let mut ids = std::collections::BTreeSet::new();
    let mut allow = Vec::new();
    for option in options {
        let option_id = bounded(&option["optionId"])?;
        if !ids.insert(option_id) {
            return Err(Error::Protocol);
        }
        if option["kind"] == "allow_once" {
            allow.push(option_id.into());
        }
    }
    Ok(Some(Request {
        key: id(&frame["id"])?,
        tool: tool.into(),
        args,
        allow,
    }))
}
pub fn response(frame: &Value) -> Result<Option<(String, Option<String>)>, Error> {
    if frame["jsonrpc"] != "2.0" || frame.get("method").is_some() || frame.get("id").is_none() {
        return Ok(None);
    }
    if frame.get("result").is_none() && frame.get("error").is_none() {
        return Ok(None);
    }
    let outcome = &frame["result"]["outcome"];
    let selected = if frame.get("error").is_none() && outcome["outcome"] == "selected" {
        Some(bounded(&outcome["optionId"])?.into())
    } else {
        None
    };
    Ok(Some((id(&frame["id"])?, selected)))
}
