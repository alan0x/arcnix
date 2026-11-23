//! Bounded session samples. Gaps are evidence of missing data, never zeroes.
use std::collections::VecDeque;

use serde::Serialize;

use crate::metrics::ResourceUsage;

pub const MAX_POINTS: usize = 1024;
pub const MAX_SECONDS: f64 = 900.0;

#[derive(Clone, Debug, Serialize)]
pub struct Point {
    /// Client monotonic seconds since the first read in this connection.
    pub at: f64,
    pub cpu: Option<f64>,
    pub memory: Option<f64>,
    pub receive: Option<f64>,
    pub transmit: Option<f64>,
    pub break_before: bool,
}

#[derive(Clone, Default)]
pub struct History {
    points: VecDeque<Point>,
    break_before: bool,
    uptime: Option<f64>,
}
fn nonnegative(value: Option<f64>) -> Option<f64> {
    value.filter(|n| n.is_finite() && *n >= 0.0)
}
impl History {
    /// Prevent joining across page changes, pauses, missing reads or reboots.
    pub fn interrupt(&mut self) {
        self.break_before = true;
        self.uptime = None;
    }
    pub fn push(&mut self, at: f64, usage: Option<&ResourceUsage>) {
        if !at.is_finite() || at < 0.0 || self.points.back().is_some_and(|p| at < p.at) {
            self.interrupt();
            return;
        }
        let uptime = usage.and_then(|u| nonnegative(u.snapshot.uptime_seconds));
        let reboot = matches!((uptime,self.uptime),(Some(now),Some(before)) if now<=before);
        let point = Point {
            at,
            cpu: usage
                .and_then(|u| nonnegative(u.cpu_percent))
                .filter(|n| *n <= 100.0),
            memory: usage
                .and_then(|u| u.snapshot.memory.as_ref())
                .and_then(|m| nonnegative(Some(m.used_percent)))
                .filter(|n| *n <= 100.0),
            receive: usage.and_then(|u| nonnegative(u.receive_bytes_per_second)),
            transmit: usage.and_then(|u| nonnegative(u.transmit_bytes_per_second)),
            break_before: self.break_before || reboot || usage.is_none(),
        };
        self.points.push_back(point);
        while self.points.len() > MAX_POINTS
            || self.points.front().is_some_and(|p| at - p.at > MAX_SECONDS)
        {
            self.points.pop_front();
        }
        self.uptime = uptime;
        self.break_before = usage.is_none();
    }
    pub fn window(&self, seconds: u16) -> Vec<Point> {
        let seconds = f64::from(seconds).min(MAX_SECONDS);
        let end = self.points.back().map_or(0.0, |p| p.at);
        self.points
            .iter()
            .filter(|p| end - p.at <= seconds)
            .cloned()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sample(uptime: f64) -> ResourceUsage {
        let snapshot = crate::metrics::ResourceSnapshot {
            uptime_seconds: Some(uptime),
            memory: Some(crate::metrics::MemoryUsage {
                total_bytes: 100,
                available_bytes: 50,
                used_bytes: 50,
                used_percent: 50.0,
            }),
            ..Default::default()
        };
        ResourceUsage {
            snapshot,
            cpu_percent: Some(30.0),
            networks: vec![],
            receive_bytes_per_second: Some(0.0),
            transmit_bytes_per_second: Some(2048.0),
        }
    }
    #[test]
    fn missing_reads_and_context_changes_break_lines_without_inventing_zeroes() {
        let mut h = History::default();
        let mut u = sample(100.0);
        u.cpu_percent = None;
        u.receive_bytes_per_second = None;
        h.push(0.0, Some(&u));
        h.push(3.0, Some(&sample(103.0)));
        h.push(6.0, None);
        h.push(9.0, Some(&sample(109.0)));
        h.interrupt();
        h.push(12.0, Some(&sample(112.0)));
        let p = h.window(60);
        assert!(p[0].cpu.is_none() && p[0].receive.is_none());
        assert_eq!(p[1].receive, Some(0.0));
        assert!(
            p[2].break_before && p[2].memory.is_none() && p[3].break_before && p[4].break_before
        );
    }
    #[test]
    fn history_has_time_and_count_bounds_and_windows_end_at_the_last_read() {
        let mut h = History::default();
        for i in 0..MAX_POINTS + 20 {
            h.push(i as f64 / 100.0, Some(&sample(100.0 + i as f64)));
        }
        assert_eq!(h.window(900).len(), MAX_POINTS);
        h.push(1000.0, Some(&sample(2000.0)));
        assert_eq!(h.window(900).len(), 1);
        h.push(1061.0, Some(&sample(2061.0)));
        assert_eq!(h.window(60).len(), 1);
        assert_eq!(h.window(300).len(), 2);
    }
    #[test]
    fn switching_windows_includes_the_boundary_and_preserves_longer_history() {
        let mut h = History::default();
        for at in [99., 100., 699., 700., 939., 940., 1000.] {
            h.push(at, Some(&sample(at)));
        }
        let times = |seconds| {
            h.window(seconds)
                .into_iter()
                .map(|p| p.at)
                .collect::<Vec<_>>()
        };
        assert_eq!(times(900), [100., 699., 700., 939., 940., 1000.]);
        assert_eq!(times(60), [940., 1000.]);
        assert_eq!(times(300), [700., 939., 940., 1000.]);
        assert_eq!(times(900), [100., 699., 700., 939., 940., 1000.]);
    }
    #[test]
    fn invalid_values_are_missing_and_uptime_resets_start_a_new_segment() {
        let mut h = History::default();
        let mut u = sample(100.0);
        u.cpu_percent = Some(f64::NAN);
        u.receive_bytes_per_second = Some(-1.0);
        u.transmit_bytes_per_second = Some(f64::INFINITY);
        u.snapshot.memory.as_mut().unwrap().used_percent = 101.0;
        h.push(0.0, Some(&u));
        h.push(3.0, Some(&sample(1.0)));
        let p = h.window(60);
        assert!(
            p[0].cpu.is_none()
                && p[0].memory.is_none()
                && p[0].receive.is_none()
                && p[0].transmit.is_none()
                && p[1].break_before
        );
        h.push(f64::NAN, None);
        h.push(-1.0, None);
        h.push(2.0, None);
        assert_eq!(h.window(60).len(), 2);
        h.push(6.0, Some(&sample(4.0)));
        assert!(h.window(60).last().unwrap().break_before);
    }
}
