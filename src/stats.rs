use crossbeam::channel::Receiver;
use crossterm::{
    cursor, execute,
    style::{self, Color, PrintStyledContent, Stylize},
    terminal::{Clear, ClearType},
};

use std::io::{self, Result, Stderr, Write};
use std::time::Instant;

use crate::timer::Timer;

pub fn stats_loop(silent: bool, receiver: Receiver<usize>) -> Result<()> {
    let mut total_bytes = 0;
    let start = Instant::now();
    let mut timer = Timer::new();
    let mut stderr = io::stderr();
    loop {
        let num_bytes = receiver.recv().unwrap();
        total_bytes += num_bytes;
        timer.tick();
        let rate_per_second = num_bytes as f64 / timer.delta.as_secs_f64();
        if !silent && timer.ready {
            timer.ready = false;
            output_progress(
                &mut stderr,
                total_bytes,
                start.elapsed().as_secs().as_clock(),
                rate_per_second,
            );
        }
        if num_bytes == 0 {
            break;
        }
    }
    if !silent {
        eprintln!();
    }
    Ok(())
}

fn output_progress(stderr: &mut Stderr, bytes: usize, elapsed: String, rate: f64) {
    let bytes = style::style(format!("{} ", bytes)).with(Color::Red);
    let elapsed = style::style(elapsed).with(Color::Green);
    let rate = style::style(format!(" [{:.0}b/s]", rate)).with(Color::Blue);
    let _ = execute!(
        stderr,
        cursor::MoveToColumn(0),
        Clear(ClearType::CurrentLine),
        PrintStyledContent(bytes),
        PrintStyledContent(elapsed),
        PrintStyledContent(rate),
    );
    let _ = stderr.flush();
}

/// The Clock trait adds a `.as_clock()` method to `u64`
///
/// # Example
/// Here is an example of how to use it.
///
/// ```rust
/// use naps::stats::Clock;
/// assert_eq!(65_u64.as_clock(), String::from("0:01:05"))
/// ```
pub trait Clock {
    fn as_clock(&self) -> String;
}

impl Clock for u64 {
    /// Renders the u64 into a time string
    fn as_clock(&self) -> String {
        let (hours, left) = (*self / 3600, *self % 3600);
        let (min, secs) = (left / 60, left % 60);
        format!("{}:{:02}:{:02}", hours, min, secs)
    }
}

#[cfg(test)]
mod tests {

    use super::Clock;

    #[test]
    fn as_time_format() {
        let pairs = vec![
            (5_u64, "0:00:05"),
            (60_u64, "0:01:00"),
            (154_u64, "0:02:34"),
            (3603_u64, "1:00:03"),
        ];

        for (input, output) in pairs {
            assert_eq!(input.as_clock().as_str(), output);
        }
    }
}
