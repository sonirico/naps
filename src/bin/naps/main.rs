use crossbeam::channel::{bounded, unbounded, Receiver};
use deno_core::futures::TryFutureExt;
use naps::read::read_loop;
use naps::write::write_loop;
use naps::{args::Args, process, stats};
use signal_hook::flag;
use std::io::Result;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;

fn main() -> Result<()> {
    let args = Args::parse();
    let shutdown = Arc::new(AtomicBool::new(false));

    flag::register(signal_hook::consts::SIGTERM, Arc::clone(&shutdown))?;

    return if args.has_script() {
        proxy_and_process(args, shutdown)
    } else {
        proxy(args, shutdown)
    };
}

fn proxy(args: Args, shutdown_arc: Arc<AtomicBool>) -> Result<()> {
    let (stats_sc, stats_rc) = unbounded();
    let (write_sc, write_rc) = bounded(1024);

    let shutdown_arc_read = Arc::clone(&shutdown_arc);
    let shutdown_arc_stats = Arc::clone(&shutdown_arc);
    let shutdown_arc_write = Arc::clone(&shutdown_arc);

    let read_handle = thread::spawn(move || {
        read_loop(
            args.source,
            args.topics,
            stats_sc,
            write_sc,
            shutdown_arc_read,
        )
    });
    let stats_handle =
        thread::spawn(move || stats::stats_loop(args.quiet, stats_rc, shutdown_arc_stats));
    let write_handle = thread::spawn(move || write_loop(args.target, write_rc, shutdown_arc_write));

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

fn proxy_and_process(args: Args, shutdown_arc: Arc<AtomicBool>) -> Result<()> {
    let Args {
        source,
        target,
        topics,
        script,
        quiet,
    } = args;

    let (stats_sc, stats_rc) = unbounded();
    let (process_sc, process_rc) = unbounded();
    let (write_sc, write_rc) = bounded(1024);

    let shutdown_arc_read = Arc::clone(&shutdown_arc);
    let shutdown_arc_process = Arc::clone(&shutdown_arc);
    let shutdown_arc_stats = Arc::clone(&shutdown_arc);
    let shutdown_arc_write = Arc::clone(&shutdown_arc);

    let read_handle = thread::Builder::new()
        .name("read".into())
        .spawn(move || read_loop(source, topics, stats_sc, process_sc, shutdown_arc_read))
        .unwrap();
    let stats_handle = thread::Builder::new()
        .name("stats".into())
        .spawn(move || stats::stats_loop(quiet, stats_rc, shutdown_arc_stats))
        .unwrap();
    let process_handle = thread::Builder::new()
        .name("process".into())
        .spawn(move || process::process_loop(script, process_rc, write_sc, shutdown_arc_process))
        .unwrap();
    let write_handle = thread::Builder::new()
        .name("write".into())
        .spawn(move || write_loop(target, write_rc, shutdown_arc_write))
        .unwrap();

    // crash if any threads have crashed
    // `.join()` returns a `thread::Result<io::Result<()>>`
    let read_io_result = read_handle.join().unwrap();
    let process_io_result = process_handle.join();
    let stats_io_result = stats_handle.join().unwrap();
    let write_io_result = write_handle.join().unwrap();

    // return an error if any thread returned an error
    read_io_result?;
    stats_io_result?;
    write_io_result?;
    process_io_result.unwrap_or_else(|e| Ok(()));

    Ok(())
}
