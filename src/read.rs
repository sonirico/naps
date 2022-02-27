use crate::msg::Msg;
use crossbeam::channel::{select, Receiver, Sender};
use nats::Message;
use std::io::Result;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::{thread, time};

pub fn read_loop(
    nats: String,
    topics: Vec<String>,
    stats_sc: Sender<u64>,
    write_sc: Sender<Msg>,
    shutdown_arc: Arc<AtomicBool>,
) -> Result<()> {
    let nc = nats::connect(nats)?;
    println!("source connected");

    for topic in topics.iter() {
        let stats = stats_sc.clone();
        let write = write_sc.clone();

        nc.subscribe(topic)?.with_handler(move |msg: Message| {
            let _ = stats.send(msg.data.len() as u64);
            let _ = write.send(Msg::new(msg.data, msg.subject));

            Ok(())
        });
    }

    let pause = time::Duration::from_secs(1);

    while !shutdown_arc.load(Ordering::Relaxed) {
        thread::sleep(pause);
    }

    eprintln!("read loop exited");

    thread::sleep(pause);

    Ok(())
}
