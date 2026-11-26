//! Coalesce layout changes before asking a remote desktop to resize.
use crate::rfb::Resolution;
use std::time::{Duration, Instant};

pub const SETTLE: Duration = Duration::from_millis(200);
const ACK_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Default)]
pub struct Sizing {
    candidate: Option<(Resolution, Instant)>,
    pending: Option<(Resolution, Instant)>,
    failed: Option<Resolution>,
    presented: bool,
}

pub struct Plan {
    pub request: Option<Resolution>,
    pub show: bool,
    pub repaint_after: Option<Duration>,
}

fn matches(a: Resolution, b: Resolution, tolerance: u16) -> bool {
    a.0.abs_diff(b.0) <= tolerance && a.1.abs_diff(b.1) <= tolerance
}

impl Sizing {
    pub fn presented(&self) -> bool {
        self.presented
    }
    pub fn plan(
        &mut self,
        now: Instant,
        desired: Resolution,
        remote: Resolution,
        image: Resolution,
        ready: bool,
        resizable: bool,
        tolerance: u16,
    ) -> Plan {
        let mut plan = Plan {
            request: None,
            show: self.presented,
            repaint_after: None,
        };
        // Saved preferences and the restored active tile must be known before
        // a default Match Tile mode can trigger any remote size changes.
        if !ready {
            self.candidate = None;
            return plan;
        }
        if self.candidate.is_none_or(|(size, _)| size != desired) {
            self.candidate = Some((desired, now));
            self.failed = None;
        }
        if let Some((size, sent)) = self.pending {
            if matches(remote, size, tolerance) {
                self.pending = None;
            } else if now.saturating_duration_since(sent) >= ACK_TIMEOUT {
                self.pending = None;
                self.failed = Some(size);
            } else {
                plan.repaint_after = Some(Duration::from_millis(50));
                return plan;
            }
        }
        let elapsed = now.saturating_duration_since(self.candidate.unwrap().1);
        if elapsed < SETTLE {
            plan.repaint_after = Some(SETTLE - elapsed);
            return plan;
        }
        if !resizable || self.failed == Some(desired) {
            self.presented = true;
        } else if matches(desired, remote, tolerance) {
            // Keep the last complete texture while the pixels for the new
            // desktop are still arriving after its size acknowledgement.
            if matches(desired, image, tolerance) {
                self.presented = true;
            } else {
                plan.repaint_after = Some(Duration::from_millis(50));
            }
        } else {
            plan.request = Some(desired);
        }
        if self.presented && !plan.show {
            plan.repaint_after = Some(Duration::ZERO);
        }
        plan.show = self.presented;
        plan
    }

    pub fn sent(&mut self, size: Resolution, now: Instant) {
        self.pending = Some((size, now));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn tick(
        sizing: &mut Sizing,
        now: Instant,
        desired: Resolution,
        remote: Resolution,
        ready: bool,
    ) -> Plan {
        sizing.plan(now, desired, remote, remote, ready, true, 0)
    }
    #[test]
    fn saved_fixed_resolution_never_resizes_through_a_temporary_tile_default() {
        let now = Instant::now();
        let fixed = Resolution(3840, 2160);
        let mut sizing = Sizing::default();
        assert!(
            tick(&mut sizing, now, Resolution(900, 700), fixed, false)
                .request
                .is_none()
        );
        assert!(
            tick(
                &mut sizing,
                now + SETTLE * 2,
                Resolution(900, 700),
                fixed,
                false
            )
            .request
            .is_none()
        );
        assert!(!tick(&mut sizing, now + SETTLE * 3, fixed, fixed, true).show);
        let plan = tick(&mut sizing, now + SETTLE * 4, fixed, fixed, true);
        assert!(plan.show && plan.request.is_none());
    }
    #[test]
    fn waits_for_stable_geometry_and_allows_only_one_outstanding_resize() {
        let now = Instant::now();
        let old = Resolution(1024, 640);
        let final_size = Resolution(1600, 900);
        let mut sizing = Sizing::default();
        tick(&mut sizing, now, Resolution(1100, 700), old, true);
        assert!(
            tick(&mut sizing, now + SETTLE / 2, final_size, old, true)
                .request
                .is_none()
        );
        assert_eq!(
            tick(&mut sizing, now + SETTLE * 2, final_size, old, true).request,
            Some(final_size)
        );
        sizing.sent(final_size, now + SETTLE * 2);
        assert!(
            tick(
                &mut sizing,
                now + SETTLE * 3,
                Resolution(1700, 1000),
                old,
                true
            )
            .request
            .is_none()
        );
        assert_eq!(
            tick(
                &mut sizing,
                now + SETTLE * 4,
                Resolution(1700, 1000),
                final_size,
                true
            )
            .request,
            Some(Resolution(1700, 1000))
        );
    }
    #[test]
    fn one_pixel_rounding_does_not_resize_but_fixed_modes_remain_exact() {
        let now = Instant::now();
        for tolerance in [0, 1] {
            let mut sizing = Sizing::default();
            let desired = Resolution(1600, 900);
            let remote = Resolution(1599, 900);
            sizing.plan(now, desired, remote, remote, true, true, tolerance);
            let plan = sizing.plan(now + SETTLE, desired, remote, remote, true, true, tolerance);
            assert_eq!(plan.request.is_none(), tolerance == 1);
        }
    }
    #[test]
    fn preserves_the_previous_image_and_does_not_repeat_an_unacknowledged_request() {
        let now = Instant::now();
        let old = Resolution(1024, 640);
        let new = Resolution(1600, 900);
        let mut sizing = Sizing::default();
        tick(&mut sizing, now, old, old, true);
        assert!(tick(&mut sizing, now + SETTLE, old, old, true).show);
        tick(&mut sizing, now + SETTLE * 2, new, old, true);
        assert_eq!(
            tick(&mut sizing, now + SETTLE * 3, new, old, true).request,
            Some(new)
        );
        sizing.sent(new, now + SETTLE * 3);
        let waiting = sizing.plan(now + SETTLE * 4, new, new, old, true, true, 0);
        assert!(waiting.show && waiting.request.is_none());
        let mut sizing = Sizing::default();
        tick(&mut sizing, now, new, old, true);
        sizing.sent(new, now);
        let timeout = tick(&mut sizing, now + ACK_TIMEOUT, new, old, true);
        assert!(timeout.show && timeout.request.is_none());
    }
}
