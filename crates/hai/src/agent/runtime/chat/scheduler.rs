use tokio::time::{Duration, Instant};

use super::attention::{Heat, Window};
use crate::{
    config::schema::AttentionConfig,
    domain::vo::{EntryAudience, StartedBy},
};

const DEBOUNCE_DURATION: Duration = Duration::from_millis(1500);

#[derive(Debug)]
pub enum Decision {
    /// Dispatch time has come; the batch of accumulated changes is the caller's to take.
    Ready(StartedBy),
    /// Not yet: the debounce is still running, heat did not win the draw, or the idle
    /// timeout has not expired.
    Defer,
    /// The ChatAgent has had no run past its timeout: idle_tick wraps the chapter up, or
    /// exits when it is empty.
    Done,
}

/// When to spend a turn, given that the accumulated changes are already in hand.
pub struct EventScheduler {
    heat: Heat,
    window: Window,
    debounce_until: Option<Instant>,
    last_run_end: Instant,
}

impl EventScheduler {
    pub fn new(cfg: &AttentionConfig) -> Self {
        Self {
            heat: Heat::new(cfg.base_attention),
            window: Window::new(cfg.window_secs),
            debounce_until: None,
            last_run_end: Instant::now(),
        }
    }

    fn refresh_heat(&mut self) {
        self.heat.decay(self.window.closes_at());
    }

    /// One change arrived, read as who it was aimed at.
    pub fn on_event(&mut self, audience: EntryAudience) {
        self.refresh_heat();

        match audience {
            EntryAudience::Agent => self.on_addressed(),
            EntryAudience::Bystanders => {
                self.debounce_until = Some(Instant::now() + DEBOUNCE_DURATION);
            }
        }
    }

    pub fn on_addressed(&mut self) {
        self.open_window();
        self.debounce_until = None;
    }

    /// Opens the attention window and leaves a running quiet period standing.
    pub fn on_spoke(&mut self) {
        self.open_window();
    }

    fn open_window(&mut self) {
        self.window.refresh();
        self.heat.reset();
    }

    pub fn next_deadline(&self, idle_timeout: Duration) -> Instant {
        if let Some(d) = self.debounce_until {
            return d;
        }

        self.last_run_end + idle_timeout
    }

    pub fn decide(&mut self, has_pending: bool, idle_timeout: Duration) -> Decision {
        let now = Instant::now();

        if let Some(d) = self.debounce_until {
            if now < d {
                return Decision::Defer;
            }
            self.debounce_until = None;
        }

        self.refresh_heat();

        if self.window.is_active() {
            if !has_pending {
                return Decision::Defer;
            }
            return Decision::Ready(StartedBy::Window);
        }

        if has_pending && self.heat.value > 0.0 && rand::random::<f64>() < self.heat.value {
            self.heat.spend();
            return Decision::Ready(StartedBy::Heat);
        }

        if now > self.last_run_end + idle_timeout {
            return Decision::Done;
        }

        Decision::Defer
    }

    /// The ChatAgent returned to Idle; the chapter's idle expiry restarts here.
    pub fn mark_run_end(&mut self) {
        self.last_run_end = Instant::now();
    }

    pub fn snapshot(&mut self) -> SchedulerStatus {
        self.refresh_heat();
        SchedulerStatus {
            heat_value: self.heat.value,
            heat_base: self.heat.base,
            window_active: self.window.is_active(),
            window_closes_in_secs: self.window.closes_in().map(|d| d.as_secs_f64()),
        }
    }
}

#[derive(Debug, Clone)]
pub struct SchedulerStatus {
    pub heat_value: f64,
    pub heat_base: f64,
    pub window_active: bool,
    pub window_closes_in_secs: Option<f64>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::schema::AttentionConfig;

    fn cfg(base: f64, window_secs: f64) -> AttentionConfig {
        AttentionConfig {
            base_attention: base,
            window_secs,
        }
    }

    /// A bystander change still has a deadline: it is not addressed, so the debounce is the only
    /// short-term deadline it gets before the window/heat path takes over.
    #[test]
    fn a_bystander_change_has_a_debounce_deadline_without_window() {
        let mut s = EventScheduler::new(&cfg(0.0, 30.0));
        s.on_event(EntryAudience::Bystanders);
        assert_eq!(
            s.next_deadline(Duration::from_secs(300)),
            s.debounce_until.expect("the debounce is armed"),
            "a bystander change's next deadline is the debounce, not the chapter's idle expiry"
        );
    }

    /// Only the end of a run moves the chapter's idle expiry: neither an addressed change
    /// (which opens the attention window) nor the window itself does.
    #[test]
    fn only_a_run_end_moves_the_idle_expiry() {
        let mut s = EventScheduler::new(&cfg(0.0, 30.0));
        s.last_run_end = Instant::now() - Duration::from_secs(100);
        let base = s.last_run_end;

        s.on_event(EntryAudience::Agent);
        assert!(
            s.window.is_active(),
            "the addressed change opened the window"
        );
        assert_eq!(s.last_run_end, base, "an event must not move the idle base");
        assert_eq!(
            s.next_deadline(Duration::from_secs(600)),
            base + Duration::from_secs(600),
            "the attention window must not stretch the idle expiry"
        );

        s.mark_run_end();
        assert!(s.last_run_end > base, "a run end moves the idle base");
    }

    /// Idle expiry is always `Done`, even with unconsumed changes: the window/heat path owns the
    /// pacing, and nothing is lost because the accumulation is the source of truth.
    #[test]
    fn decide_done_on_idle_expiry_even_with_pending_changes() {
        let mut s = EventScheduler::new(&cfg(0.0, 30.0));
        s.on_event(EntryAudience::Bystanders);
        s.debounce_until = None;
        s.last_run_end = Instant::now() - Duration::from_secs(601);

        match s.decide(true, Duration::from_secs(600)) {
            Decision::Done => {}
            other => {
                panic!(
                    "idle expiry must always be Done, even with unconsumed changes; got {other:?}"
                )
            }
        }
    }

    /// A heat hit with nothing accumulated defers: an empty batch must never produce an empty turn.
    #[test]
    fn no_empty_run_on_heat_hit() {
        let mut s = EventScheduler::new(&cfg(0.0, 30.0));
        s.heat.value = 1.0; // force a heat hit
        match s.decide(false, Duration::from_secs(300)) {
            Decision::Defer => {}
            other => panic!("a heat hit with an empty batch must Defer; got {other:?}"),
        }
    }

    /// An empty batch defers even inside the open window; the retry budget relies on it.
    #[test]
    fn no_pending_defers_inside_the_window() {
        let mut s = EventScheduler::new(&cfg(0.0, 30.0));
        s.on_event(EntryAudience::Agent);
        assert!(
            s.window.is_active(),
            "the addressed change opened the window"
        );

        match s.decide(false, Duration::from_secs(300)) {
            Decision::Defer => {}
            other => panic!("an empty batch must Defer inside the window; got {other:?}"),
        }
        assert!(
            matches!(s.decide(true, Duration::from_secs(300)), Decision::Ready(_)),
            "a pending batch inside the window is Ready"
        );
    }

    /// The two window openers differ in exactly one respect: an addressed change also drops a quiet
    /// period a bystander armed, while the agent speaking leaves it running.
    #[test]
    fn an_addressed_change_clears_the_quiet_period_and_speaking_does_not() {
        let mut s = EventScheduler::new(&cfg(0.05, 30.0));

        s.on_event(EntryAudience::Bystanders);
        assert!(
            s.debounce_until.is_some(),
            "the bystander arrival armed a quiet period"
        );

        s.on_spoke();
        assert!(s.window.is_active(), "speaking opens the window");
        assert!(
            s.debounce_until.is_some(),
            "speaking must not cut a running quiet period short"
        );

        s.on_addressed();
        assert!(
            s.debounce_until.is_none(),
            "an addressed change clears the quiet period"
        );
    }

    /// Which path let a dispatch through is what the turn record carries.
    #[test]
    fn the_ready_ground_names_the_path() {
        let mut s = EventScheduler::new(&cfg(0.0, 30.0));
        s.on_addressed();
        match s.decide(true, Duration::from_secs(300)) {
            Decision::Ready(StartedBy::Window) => {}
            other => panic!("an open window dispatches with the window ground; got {other:?}"),
        }

        let mut s = EventScheduler::new(&cfg(1.0, 0.0));
        match s.decide(true, Duration::from_secs(300)) {
            Decision::Ready(StartedBy::Heat) => {}
            other => panic!("a heat hit dispatches with the heat ground; got {other:?}"),
        }
    }
}
