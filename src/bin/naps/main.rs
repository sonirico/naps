use crossbeam::channel::{bounded, unbounded};
use crossbeam::channel::{Receiver, Sender};
use naps::{args::Args, stats};
use nats::Message;
use std::io::{ErrorKind, Result};
use std::{thread, time};

pub struct Msg {
    data: Vec<u8>,
    topic: String,
}

impl Msg {
    pub fn new(data: Vec<u8>, topic: String) -> Self {
        Self { topic, data }
    }
}

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

fn main() -> Result<()> {
    let args = Args::parse();
    let Args {
        source,
        target,
        topics,
        quiet,
    } = args;

    let (stats_sc, stats_rc) = unbounded();
    let (write_sc, write_rc) = bounded(1024);

    let read_handle = thread::spawn(move || read_loop(source, topics, stats_sc, write_sc));
    let stats_handle = thread::spawn(move || stats::stats_loop(quiet, stats_rc));
    let write_handle = thread::spawn(move || write_loop(target, write_rc));
    // crash if any threads have crashed
    // `.join()` returns a `thread::Result<io::Result<()>>`
    let read_io_result = read_handle.join().unwrap();
    let stats_io_result = stats_handle.join().unwrap();
    let write_io_result = write_handle.join().unwrap();

    // return an error if any thread returned an error
    read_io_result?;
    stats_io_result?;
    write_io_result?;

    Ok(())
}
