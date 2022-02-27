use crate::msg::Msg;
use crossbeam::channel::Receiver;
use crossbeam::select;
use std::io::{ErrorKind, Result};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

pub fn write_loop(
    nats: String,
    msg_rc: Receiver<Msg>,
    shutdown_arc: Arc<AtomicBool>,
) -> Result<()> {
    let nc = nats::connect(nats)?;
    println!("target connected");

    while !shutdown_arc.load(Ordering::Relaxed) {
        let msg = msg_rc.recv().unwrap();

        if let Err(e) = nc.publish(&msg.topic, &msg.data) {
            println!("{}", e);
            if e.kind() == ErrorKind::ConnectionAborted {
                return Err(e);
            }
        }
    }

    eprintln!("write loop exited");

    Ok(())
}
