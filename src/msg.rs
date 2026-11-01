use std::collections::BTreeMap;

use bytes::Bytes;

#[derive(Debug, Clone, PartialEq)]
pub struct Msg {
    pub subject: String,
    pub headers: BTreeMap<String, Vec<String>>,
    pub data: Bytes,
}
