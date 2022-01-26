use std::time::{Duration, Instant};

pub struct Timer {
    pub last: Instant,
    pub delta: Duration,
    pub period: Duration,
    pub countdown: Duration,
    pub ready: bool,
}

impl Timer {
    pub fn new() -> Self {
        let now = Instant::now();
        Self {
            last: now,
            delta: Duration::default(),
            period: Duration::from_millis(1000),
            countdown: Duration::default(),
            ready: true,
        }
    }

    pub fn tick(&mut self) {
        let now = Instant::now();
        self.delta = now - self.last;
        self.last = now;
        self.countdown = self.countdown.checked_sub(self.delta).unwrap_or_else(|| {
            self.ready = true;
            self.period
        })
    }
}

impl Default for Timer {
    fn default() -> Self {
        Self::new()
    }
}
