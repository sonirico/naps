pub struct Msg {
    pub data: Vec<u8>,
    pub topic: String,
}

impl Msg {
    pub fn new(data: Vec<u8>, topic: String) -> Self {
        Self { topic, data }
    }
}
