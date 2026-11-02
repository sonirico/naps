use std::collections::BTreeMap;

use extism_pdk::*;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
struct Msg {
    #[serde(skip_serializing_if = "Option::is_none")]
    subject: Option<String>,
    headers: BTreeMap<String, Vec<String>>,
    data: String,
}

#[plugin_fn]
pub fn recv(input: Vec<u8>) -> FnResult<Vec<u8>> {
    let mut msg: Msg = serde_json::from_slice(&input)?;
    if msg.headers.contains_key("x-fail") {
        return Err(Error::msg("boom").into());
    }
    if let Some(subject) = msg.headers.get("x-rename").and_then(|v| v.first()) {
        msg.subject = Some(subject.clone());
    } else {
        msg.subject = None;
    }
    if msg.headers.contains_key("x-fanout") {
        let two = [&msg, &msg];
        return Ok(serde_json::to_vec(&two)?);
    }
    Ok(serde_json::to_vec(&msg)?)
}
