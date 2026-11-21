//! Cursor blinking.

use std::time::{Duration, Instant};

/// How the cursor blinks while blinking is allowed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timing {
    /// Time the cursor stays shown, and then hidden.
    pub interval: Duration,
    /// Blinking stops (the cursor stays shown) this long after the last
    /// input; `None` blinks forever.
    pub timeout: Option<Duration>,
}

/// The blink phase of the cursor.
#[derive(Clone, Copy, Debug)]
pub struct Blink {
    visible: bool,
    next_flip: Instant,
    last_input: Instant,
}

impl Blink {
    pub fn new(now: Instant) -> Self {
        Self {
            visible: true,
            next_flip: now,
            last_input: now,
        }
    }

    pub fn visible(&self) -> bool {
        self.visible
    }

    /// Input shows the cursor at once and restarts the timeout, so the
    /// cursor does not vanish under the text being typed.
    pub fn reset(&mut self, now: Instant, timing: Option<Timing>) {
        self.visible = true;
        self.last_input = now;
        if let Some(timing) = timing {
            self.next_flip = now + timing.interval;
        }
    }

    fn timed_out(&self, now: Instant, timing: Timing) -> bool {
        timing
            .timeout
            .is_some_and(|timeout| now >= self.last_input + timeout)
    }

    /// Moves the phase on to `now`; `timing` is `None` while the cursor
    /// should not blink. Returns whether the cursor appeared or vanished.
    pub fn update(&mut self, now: Instant, timing: Option<Timing>) -> bool {
        let was = self.visible;
        match timing {
            Some(timing) if !self.timed_out(now, timing) => {
                if now >= self.next_flip {
                    self.visible = !self.visible;
                    // Not `+= interval`: after a long sleep that would flip
                    // once per missed interval.
                    self.next_flip = now + timing.interval;
                }
            }
            _ => self.visible = true,
        }
        self.visible != was
    }

    /// When [`Blink::update`] next has something to do; `None` while the
    /// cursor stays as it is.
    pub fn deadline(&self, now: Instant, timing: Option<Timing>) -> Option<Instant> {
        match timing {
            Some(timing) if !self.timed_out(now, timing) => Some(self.next_flip),
            // The cursor may still be in its hidden phase.
            _ if !self.visible => Some(now),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TIMING: Timing = Timing {
        interval: Duration::from_millis(500),
        timeout: Some(Duration::from_secs(5)),
    };

    fn ms(t0: Instant, n: u64) -> Instant {
        t0 + Duration::from_millis(n)
    }

    #[test]
    fn the_cursor_alternates_every_interval() {
        let t0 = Instant::now();
        let mut blink = Blink::new(t0);
        blink.reset(t0, Some(TIMING));
        assert!(!blink.update(ms(t0, 499), Some(TIMING)));
        assert!(blink.visible());
        assert!(blink.update(ms(t0, 500), Some(TIMING)));
        assert!(!blink.visible());
        assert_eq!(
            blink.deadline(ms(t0, 500), Some(TIMING)),
            Some(ms(t0, 1000))
        );
        assert!(blink.update(ms(t0, 1000), Some(TIMING)));
        assert!(blink.visible());
    }

    #[test]
    fn input_shows_the_cursor_and_restarts_the_interval() {
        let t0 = Instant::now();
        let mut blink = Blink::new(t0);
        blink.reset(t0, Some(TIMING));
        blink.update(ms(t0, 500), Some(TIMING));
        assert!(!blink.visible());
        blink.reset(ms(t0, 600), Some(TIMING));
        assert!(blink.visible());
        assert!(!blink.update(ms(t0, 1000), Some(TIMING)));
        assert!(blink.update(ms(t0, 1100), Some(TIMING)));
    }

    #[test]
    fn blinking_stops_with_the_cursor_shown_after_the_timeout() {
        let t0 = Instant::now();
        let mut blink = Blink::new(t0);
        blink.reset(t0, Some(TIMING));
        blink.update(ms(t0, 4900), Some(TIMING));
        assert!(!blink.visible());
        assert!(blink.update(ms(t0, 5000), Some(TIMING)));
        assert!(blink.visible());
        assert_eq!(blink.deadline(ms(t0, 5000), Some(TIMING)), None);
        assert!(!blink.update(ms(t0, 5500), Some(TIMING)));
    }

    #[test]
    fn without_a_timeout_it_blinks_forever() {
        let timing = Timing {
            timeout: None,
            ..TIMING
        };
        let t0 = Instant::now();
        let mut blink = Blink::new(t0);
        blink.reset(t0, Some(timing));
        assert!(blink.update(ms(t0, 60_000), Some(timing)));
        assert!(blink.deadline(ms(t0, 60_000), Some(timing)).is_some());
    }

    #[test]
    fn turning_blinking_off_shows_the_cursor() {
        let t0 = Instant::now();
        let mut blink = Blink::new(t0);
        blink.reset(t0, Some(TIMING));
        blink.update(ms(t0, 500), Some(TIMING));
        assert_eq!(blink.deadline(ms(t0, 600), None), Some(ms(t0, 600)));
        assert!(blink.update(ms(t0, 600), None));
        assert!(blink.visible());
        assert_eq!(blink.deadline(ms(t0, 600), None), None);
    }
}
