use serde::Serialize;

pub(super) fn encoded_frame_hex(frame: &impl Serialize) -> Result<String, String> {
    let payload = serde_json::to_vec(frame).map_err(|error| error.to_string())?;
    let length = u32::try_from(payload.len()).map_err(|_| "frame exceeds u32 framing limit")?;
    let mut encoded = length.to_be_bytes().to_vec();
    encoded.extend(payload);
    let mut hex = String::with_capacity(encoded.len() * 2);
    for byte in encoded {
        use std::fmt::Write as _;
        let _ = write!(&mut hex, "{byte:02x}");
    }
    Ok(hex)
}
