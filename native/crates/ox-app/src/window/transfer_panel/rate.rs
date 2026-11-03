// SPDX-License-Identifier: AGPL-3.0-only
//! The speed, time remaining and items remaining of an action, worded as
//! the Windows progress window words them ("Speed: 32.8 MB/s", "Time
//! remaining: About 25 seconds", "Items remaining: 1,030 (313 MB)"),
//! OPS-021. Time the user paused the action does not count.
//!
//! The speed is the recent one, over the last few seconds, as the graph
//! shows it; the time remaining divides what is left by the average
//! speed so far, so it does not jump with each file. It says
//! "Calculating…" until a quarter of a second has passed, as the Windows
//! UX guidance asks not to show an estimate before it can be made.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use ox_core::format::pretty_bytes;
use ox_core::transfer::ByteProgress;

/// How far back the recent speed looks.
const SPEED_WINDOW: Duration = Duration::from_secs(3);
/// How long before the first estimate.
const SETTLING: Duration = Duration::from_millis(250);

/// What an action's bytes say at one report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Reading {
    /// Bytes per second over the last few seconds; `None` until known.
    pub(super) speed: Option<u64>,
    /// The time left; `None` while it is being calculated, or when the
    /// total is unknown.
    pub(super) time_left: Option<Duration>,
    /// The bytes left, when the free-space check measured the total.
    pub(super) bytes_left: Option<u64>,
    /// The bytes written so far.
    pub(super) written: u64,
    /// Whether the total is known, so a time can be estimated.
    pub(super) knows_total: bool,
}

/// The clock of one action, which stops while it is paused.
#[derive(Debug, Default)]
pub(super) struct Rate {
    started: Option<Instant>,
    paused: Option<Instant>,
    pause_time: Duration,
    /// Recent (running time, bytes written) pairs, oldest first.
    samples: VecDeque<(Duration, u64)>,
}

impl Rate {
    /// Reads a byte report.
    pub(super) fn report(&mut self, bytes: ByteProgress) -> Reading {
        let elapsed = self.running_time();
        while self
            .samples
            .front()
            .is_some_and(|(time, _)| elapsed.saturating_sub(*time) > SPEED_WINDOW)
        {
            self.samples.pop_front();
        }
        self.samples.push_back((elapsed, bytes.batch_written));
        let recent = self
            .samples
            .front()
            .copied()
            .unwrap_or((elapsed, bytes.batch_written));
        reading(bytes, elapsed, recent)
    }

    pub(super) fn pause(&mut self) {
        if self.paused.is_none() {
            self.paused = Some(Instant::now());
        }
    }

    pub(super) fn resume(&mut self) {
        if let Some(paused) = self.paused.take() {
            self.pause_time += paused.elapsed();
        }
    }

    /// How long the action has run, without its pauses.
    fn running_time(&mut self) -> Duration {
        let now = Instant::now();
        let start = *self.started.get_or_insert(now);
        self.paused
            .unwrap_or(now)
            .saturating_duration_since(start)
            .saturating_sub(self.pause_time)
    }
}

/// What `bytes` say after `elapsed` running time, `recent` being the
/// (running time, bytes) of the oldest recent report.
fn reading(bytes: ByteProgress, elapsed: Duration, recent: (Duration, u64)) -> Reading {
    let total = bytes.batch_size.filter(|total| *total >= bytes.batch_written);
    let bytes_left = total.map(|total| total - bytes.batch_written);
    let settled = elapsed >= SETTLING && bytes.batch_written > 0;
    let average = settled
        .then(|| per_second(bytes.batch_written, elapsed))
        .filter(|speed| *speed > 0);
    let span = elapsed.saturating_sub(recent.0);
    let speed = if span >= SETTLING {
        Some(per_second(bytes.batch_written.saturating_sub(recent.1), span))
    } else {
        average
    };
    let time_left = bytes_left
        .zip(average)
        .map(|(left, speed)| Duration::from_secs(left.div_ceil(speed)));
    Reading {
        speed,
        time_left,
        bytes_left,
        written: bytes.batch_written,
        knows_total: total.is_some(),
    }
}

/// `bytes` per second over `time`, in integers wide enough for any file.
fn per_second(bytes: u64, time: Duration) -> u64 {
    let millis = time.as_millis().max(1);
    u64::try_from(u128::from(bytes) * 1000 / millis).unwrap_or(u64::MAX)
}

/// "Speed: 32.8 MB/s".
pub(super) fn speed_text(speed: u64) -> String {
    ox_core::i18n::format_message("Speed: {size}/s", &[("size", &pretty_bytes(speed))])
}

/// "Time remaining: About 25 seconds", or "Calculating…" before an
/// estimate can be made.
pub(super) fn time_text(time_left: Option<Duration>) -> String {
    let time = time_left.map_or_else(
        || ox_core::i18n::gettext("Calculating…"),
        |time| about(time.as_secs()),
    );
    ox_core::i18n::format_message("Time remaining: {time}", &[("time", &time)])
}

/// "About 4 minutes and 30 seconds": the largest unit and the next, as
/// Windows words it, without seconds once it is an hour or more.
fn about(seconds: u64) -> String {
    use ox_core::i18n::ngettext;
    let count = |text: String, count: u64| text.replace("{count}", &count.to_string());
    let hours = |n| count(ngettext("{count} hour", "{count} hours", n), n);
    let minutes = |n| count(ngettext("{count} minute", "{count} minutes", n), n);
    let seconds_text = |n| count(ngettext("{count} second", "{count} seconds", n), n);
    let (larger, smaller) = match seconds {
        0..60 => (seconds_text(seconds), None),
        60..3600 => (
            minutes(seconds / 60),
            (!seconds.is_multiple_of(60)).then(|| seconds_text(seconds % 60)),
        ),
        _ => (
            hours(seconds / 3600),
            (seconds % 3600 >= 60).then(|| minutes(seconds % 3600 / 60)),
        ),
    };
    let time = match smaller {
        Some(smaller) => ox_core::i18n::format_message(
            "{larger} and {smaller}",
            &[("larger", &larger), ("smaller", &smaller)],
        ),
        None => larger,
    };
    ox_core::i18n::format_message("About {time}", &[("time", &time)])
}

/// "Items remaining: 1,030 (313 MB)"; with the bytes processed instead
/// when the total size is unknown, and only the count without bytes.
pub(super) fn items_text(left: u64, reading: Option<&Reading>) -> String {
    let count = grouped(left);
    let size = reading.map(|reading| match reading.bytes_left {
        Some(bytes) => pretty_bytes(bytes),
        None => {
            ox_core::i18n::format_message("{size} processed", &[("size", &pretty_bytes(reading.written))])
        }
    });
    match size {
        Some(size) => ox_core::i18n::format_message(
            "Items remaining: {count} ({size})",
            &[("count", &count), ("size", &size)],
        ),
        None => ox_core::i18n::format_message("Items remaining: {count}", &[("count", &count)]),
    }
}

/// `count` with commas between thousands: 1718 becomes "1,718".
pub(super) fn grouped(count: u64) -> String {
    let digits = count.to_string();
    let mut grouped = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    grouped
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes(written: u64, size: Option<u64>) -> ByteProgress {
        ByteProgress {
            file_written: 0,
            file_size: 0,
            batch_written: written,
            batch_size: size,
            file_started: false,
        }
    }

    /// The speed, the time left and the bytes left come from the bytes
    /// written; an unknown total is never invented, and no estimate is
    /// made before there is something to go on.
    ///
    /// parity: OPS-021
    #[test]
    fn progress_reports_batch_bytes_speed_and_remaining_without_inventing_unknown_totals() {
        let two_seconds = Duration::from_secs(2);
        let known = reading(bytes(2048, Some(8192)), two_seconds, (Duration::ZERO, 0));
        let unknown = reading(bytes(2048, None), two_seconds, (Duration::ZERO, 0));
        let early = reading(
            bytes(2048, Some(8192)),
            Duration::from_millis(100),
            (Duration::ZERO, 0),
        );

        assert_eq!(known.speed, Some(1024));
        assert_eq!(known.time_left, Some(Duration::from_secs(6)));
        assert_eq!(known.bytes_left, Some(6144));
        assert_eq!((unknown.bytes_left, unknown.time_left), (None, None));
        assert!(!unknown.knows_total && unknown.speed == Some(1024));
        assert_eq!(early.time_left, None, "too early to estimate");
        assert_eq!(time_text(early.time_left), "Time remaining: Calculating…");
    }

    /// The speed is that of the last few seconds, not the average.
    #[test]
    fn the_speed_is_the_recent_one() {
        let slow_start = reading(
            bytes(11 * 1024 * 1024, None),
            Duration::from_secs(10),
            (Duration::from_secs(8), 1024 * 1024),
        );

        assert_eq!(slow_start.speed, Some(5 * 1024 * 1024));
        assert_eq!(speed_text(34_393_293), "Speed: 32.8 MB/s");
    }

    /// The time left reads as Windows says it: the largest unit and the
    /// next, never seconds once it is an hour.
    ///
    /// parity: OPS-021
    #[test]
    fn time_remaining_reads_as_windows_says_it() {
        let time = |seconds| time_text(Some(Duration::from_secs(seconds)));

        assert_eq!(time(25), "Time remaining: About 25 seconds");
        assert_eq!(time(1), "Time remaining: About 1 second");
        assert_eq!(time(60), "Time remaining: About 1 minute");
        assert_eq!(time(270), "Time remaining: About 4 minutes and 30 seconds");
        assert_eq!(time(3900), "Time remaining: About 1 hour and 5 minutes");
        assert_eq!(time(7230), "Time remaining: About 2 hours");
    }

    /// The items left come with the bytes left, or the bytes processed
    /// when the total is unknown.
    ///
    /// parity: OPS-021
    #[test]
    fn items_remaining_carry_their_size() {
        let known = reading(bytes(0, Some(328_204_288)), Duration::ZERO, (Duration::ZERO, 0));
        let unknown = reading(bytes(5 * 1024 * 1024, None), Duration::ZERO, (Duration::ZERO, 0));

        assert_eq!(items_text(1030, Some(&known)), "Items remaining: 1,030 (313 MB)");
        assert_eq!(
            items_text(3, Some(&unknown)),
            "Items remaining: 3 (5.0 MB processed)"
        );
        assert_eq!(items_text(24_850, None), "Items remaining: 24,850");
    }
}
