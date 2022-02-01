use crate::msg::Msg;
use crossbeam::channel::Sender;
use nats::Message;
use std::io::Result;
use std::{thread, time};

pub fn read_loop(
    nats: String,
    topics: Vec<String>,
    stats_sc: Sender<usize>,
    write_sc: Sender<Msg>,
) -> Result<()> {
    let nc = nats::connect(nats)?;
    println!("source connected");

    for topic in topics.iter() {
        let stats = stats_sc.clone();
        let write = write_sc.clone();
        nc.subscribe(topic)?.with_handler(move |msg: Message| {
            let _ = stats.send(msg.data.len());
            let _ = write.send(Msg::new(msg.data, msg.subject));

            Ok(())
        });
    }

    let pause = time::Duration::from_secs(1);

    loop {
        thread::sleep(pause);
    }
}
