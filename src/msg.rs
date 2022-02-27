use std::fmt::{Display, Formatter};

#[derive(Debug)]
pub struct Msg {
    pub data: Vec<u8>,
    pub topic: String,
}

impl Msg {
    pub fn new(data: Vec<u8>, topic: String) -> Self {
        Self { topic, data }
    }

    pub fn from_str(data: String, topic: String) -> Self {
        Self {
            topic,
            data: data.into_bytes(),
        }
    }
}

impl Display for Msg {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.topic)
    }
}
