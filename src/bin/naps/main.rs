use crossbeam::channel::{bounded, unbounded};
use naps::read::read_loop;
use naps::write::write_loop;
use naps::{args::Args, stats};
use std::io::Result;
use std::thread;

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
