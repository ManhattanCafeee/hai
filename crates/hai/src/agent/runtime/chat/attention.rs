use tokio::time::{Duration, Instant};

const DECAY_STEP: f64 = 60.0;
const MAX_HEAT: f64 = 1.0;
const SPEND_HEAT: f64 = 0.25;

pub(super) struct Heat {
    pub value: f64,
    pub base: f64,
    last_decay: Instant,
}

impl Heat {
    pub fn new(base: f64) -> Self {
        Self {
            value: base,
            base,
            last_decay: Instant::now(),
        }
    }

    pub fn decay(&mut self, window_closes_at: Option<Instant>) {
        let now = Instant::now();
        let mut start = self.last_decay;

        if let Some(end) = window_closes_at
            && end > start
        {
            start = end;
        }

        if now <= start {
            self.last_decay = now;
            return;
        }

        let elapsed = now.duration_since(start).as_secs_f64();
        let steps = (elapsed / DECAY_STEP).floor() as i32;

        if steps > 0 {
            let excess = self.value - self.base;
            if excess > 0.0 {
                self.value = self.base + excess * 0.5_f64.powi(steps);
            }
            self.last_decay = start + Duration::from_secs_f64(steps as f64 * DECAY_STEP);
        }
    }

    pub fn spend(&mut self) {
        self.value = (self.value - SPEND_HEAT).max(self.base);
    }

    pub fn reset(&mut self) {
        self.value = MAX_HEAT;
        self.last_decay = Instant::now();
    }
}

pub(super) struct Window {
    start: Option<Instant>,
    duration_secs: f64,
}

impl Window {
    pub fn new(duration_secs: f64) -> Self {
        Self {
            start: None,
            duration_secs,
        }
    }

    pub fn is_active(&self) -> bool {
        self.start
            .map(|t| t.elapsed().as_secs_f64() < self.duration_secs)
            .unwrap_or(false)
    }

    pub fn closes_at(&self) -> Option<Instant> {
        self.start
            .map(|t| t + Duration::from_secs_f64(self.duration_secs))
    }

    pub fn refresh(&mut self) {
        self.start = Some(Instant::now());
    }

    pub fn closes_in(&self) -> Option<Duration> {
        self.closes_at()
            .map(|t| t.saturating_duration_since(Instant::now()))
    }
}
