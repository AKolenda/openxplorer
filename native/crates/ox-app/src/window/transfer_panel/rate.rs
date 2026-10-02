// SPDX-License-Identifier: AGPL-3.0-only
//! Transfer byte rates exclude time the user paused the job.
use ox_core::format::pretty_bytes;
use ox_core::transfer::ByteProgress;
use std::time::{Duration, Instant};

#[derive(Debug, Default)]
pub(super) struct Rate {
    started: Option<Instant>,
    paused: Option<Instant>,
    pause_time: Duration,
}

impl Rate {
    pub(super) fn report(&mut self, bytes: ByteProgress) -> String {
        let now = Instant::now();
        let start = *self.started.get_or_insert(now);
        let elapsed = self
            .paused
            .unwrap_or(now)
            .saturating_duration_since(start)
            .saturating_sub(self.pause_time);
        describe(bytes, elapsed)
    }
    pub(super) fn pause(&mut self) {
        self.paused = Some(Instant::now());
    }
    pub(super) fn resume(&mut self) {
        if let Some(paused) = self.paused.take() {
            self.pause_time += paused.elapsed();
        }
    }
}

fn describe(bytes: ByteProgress, elapsed: Duration) -> String {
    let done = pretty_bytes(bytes.batch_written);
    let size = bytes
        .batch_size
        .filter(|size| *size >= bytes.batch_written)
        .map_or_else(
            || format!("{done} processed"),
            |total| format!("{done} / {}", pretty_bytes(total)),
        );
    if elapsed < Duration::from_millis(250) || bytes.batch_written == 0 {
        return format!("{size} · Estimating…");
    }
    // Integer arithmetic keeps the estimate bounded for very large files.
    let millis = elapsed.as_millis().max(1);
    let speed = u64::try_from(u128::from(bytes.batch_written) * 1000 / millis).unwrap_or(u64::MAX);
    if speed == 0 {
        return format!("{size} · Estimating…");
    }
    let rate = format!("{size} · {}/s", pretty_bytes(speed));
    bytes
        .batch_size
        .filter(|total| *total >= bytes.batch_written)
        .map_or(rate.clone(), |total| {
            let seconds = total.saturating_sub(bytes.batch_written).div_ceil(speed);
            format!("{rate} · {}m {}s remaining", seconds / 60, seconds % 60)
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    /// parity: OPS-021
    #[test]
    fn progress_reports_batch_bytes_speed_and_remaining_without_inventing_unknown_totals() {
        let bytes = ByteProgress {
            file_written: 1024,
            file_size: 4096,
            batch_written: 2048,
            batch_size: Some(8192),
        };
        let known = describe(bytes, Duration::from_secs(2));
        assert!(
            known.contains("/s") && known.contains("0m 6s remaining"),
            "{known}"
        );
        let unknown = describe(
            ByteProgress {
                batch_size: None,
                ..bytes
            },
            Duration::from_secs(2),
        );
        assert!(unknown.contains("processed") && !unknown.contains("remaining"));
    }
}
