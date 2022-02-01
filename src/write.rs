use crate::msg::Msg;
use crossbeam::channel::Receiver;
use std::io::{ErrorKind, Result};

pub fn write_loop(nats: String, receiver: Receiver<Msg>) -> Result<()> {
    let nc = nats::connect(nats)?;
    println!("target connected");

    loop {
        let msg = receiver.recv().unwrap();

        if let Err(e) = nc.publish(&msg.topic, &msg.data) {
            println!("{}", e);
            if e.kind() == ErrorKind::ConnectionAborted {
                return Err(e);
            }
        }
    }
}
