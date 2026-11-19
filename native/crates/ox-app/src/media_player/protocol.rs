// SPDX-License-Identifier: AGPL-3.0-only
//! What the app and its player process say to each other, one line each:
//! the app's commands on the player's standard input, and the player's
//! events on its standard output, which start with [`HELLO`]. A frame's line is followed by its
//! pixels, RGBA rows of `stride` bytes. Anything else is a broken player,
//! which the app stops.

/// The widest or tallest frame the app takes: four times the largest
/// preview, which no player asked for a preview sends.
pub(crate) const MAX_FRAME_SIDE: u32 = 4096;
/// The player's first line: whatever came before it on its output, such
/// as a test harness's header, is not the player's.
pub(crate) const HELLO: &str = "ox-player 1\n";
/// The longest line either side reads.
pub(crate) const MAX_LINE: u64 = 512;

/// What the app asks of the player.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Command {
    Play,
    Pause,
    /// To this moment, in microseconds.
    Seek(i64),
    Volume {
        volume: f64,
        muted: bool,
    },
    /// Pause, stop the pipeline and exit.
    Quit,
}

impl Command {
    pub(crate) fn line(self) -> String {
        match self {
            Self::Play => "play\n".to_owned(),
            Self::Pause => "pause\n".to_owned(),
            Self::Seek(micros) => format!("seek {micros}\n"),
            Self::Volume { volume, muted } => format!("volume {volume} {}\n", u8::from(muted)),
            Self::Quit => "quit\n".to_owned(),
        }
    }

    pub(crate) fn parse(line: &str) -> Option<Self> {
        let mut words = line.split_ascii_whitespace();
        let command = match words.next()? {
            "play" => Self::Play,
            "pause" => Self::Pause,
            "seek" => Self::Seek(words.next()?.parse().ok().filter(|micros| *micros >= 0)?),
            "volume" => Self::Volume {
                volume: words
                    .next()?
                    .parse::<f64>()
                    .ok()
                    .filter(|volume| (0.0..=1.0).contains(volume))?,
                muted: flag(words.next()?)?,
            },
            "quit" => Self::Quit,
            _ => return None,
        };
        words.next().is_none().then_some(command)
    }
}

/// What the player tells the app.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Event {
    /// The stream is ready to play: its length in microseconds (0 when
    /// unknown), and what it has.
    Ready {
        duration: i64,
        seekable: bool,
        has_audio: bool,
        has_video: bool,
    },
    /// Where playback is, in microseconds.
    Position(i64),
    Seeked,
    SeekFailed,
    Ended,
    /// The recording cannot be played; the text is `GStreamer`'s.
    Failed(String),
    /// The pixels of a frame follow: `height` rows of `stride` bytes.
    Frame {
        width: u32,
        height: u32,
        stride: u32,
    },
}

impl Event {
    pub(crate) fn line(&self) -> String {
        match self {
            Self::Ready {
                duration,
                seekable,
                has_audio,
                has_video,
            } => format!(
                "ready {duration} {} {} {}\n",
                u8::from(*seekable),
                u8::from(*has_audio),
                u8::from(*has_video)
            ),
            Self::Position(micros) => format!("position {micros}\n"),
            Self::Seeked => "seeked\n".to_owned(),
            Self::SeekFailed => "seek-failed\n".to_owned(),
            Self::Ended => "ended\n".to_owned(),
            // One line, however GStreamer worded it.
            Self::Failed(text) => {
                let text: String = text.chars().filter(|c| !c.is_control()).take(200).collect();
                format!("failed {text}\n")
            }
            Self::Frame {
                width,
                height,
                stride,
            } => format!("frame {width} {height} {stride}\n"),
        }
    }

    pub(crate) fn parse(line: &str) -> Option<Self> {
        let line = line.strip_suffix('\n').unwrap_or(line);
        if let Some(text) = line.strip_prefix("failed ") {
            return Some(Self::Failed(text.to_owned()));
        }
        let mut words = line.split_ascii_whitespace();
        let event = match words.next()? {
            "ready" => Self::Ready {
                duration: words.next()?.parse().ok().filter(|micros| *micros >= 0)?,
                seekable: flag(words.next()?)?,
                has_audio: flag(words.next()?)?,
                has_video: flag(words.next()?)?,
            },
            "position" => Self::Position(words.next()?.parse().ok().filter(|micros| *micros >= 0)?),
            "seeked" => Self::Seeked,
            "seek-failed" => Self::SeekFailed,
            "ended" => Self::Ended,
            "frame" => Self::Frame {
                width: words.next()?.parse().ok()?,
                height: words.next()?.parse().ok()?,
                stride: words.next()?.parse().ok()?,
            },
            _ => return None,
        };
        words.next().is_none().then_some(event)
    }
}

/// The bytes of a frame of `width` by `height` pixels with rows of
/// `stride` bytes, if the app takes such a frame.
pub(crate) fn frame_length(width: u32, height: u32, stride: u32) -> Option<usize> {
    let fits = (1..=MAX_FRAME_SIDE).contains(&width) && (1..=MAX_FRAME_SIDE).contains(&height);
    let row = width.checked_mul(4)?;
    // GStreamer pads a row to at most a multiple of 64 bytes.
    let padded = fits && stride >= row && stride - row < 64;
    padded.then(|| usize::try_from(u64::from(stride) * u64::from(height)).ok())?
}

fn flag(word: &str) -> Option<bool> {
    match word {
        "0" => Some(false),
        "1" => Some(true),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_and_events_read_back_as_written() {
        for command in [
            Command::Play,
            Command::Pause,
            Command::Seek(1_500_000),
            Command::Volume {
                volume: 0.25,
                muted: true,
            },
            Command::Quit,
        ] {
            assert_eq!(Command::parse(&command.line()), Some(command));
        }
        for event in [
            Event::Ready {
                duration: 20_000_000,
                seekable: true,
                has_audio: true,
                has_video: false,
            },
            Event::Position(42),
            Event::Seeked,
            Event::SeekFailed,
            Event::Ended,
            Event::Failed("Could not demultiplex stream.".to_owned()),
            Event::Frame {
                width: 216,
                height: 162,
                stride: 864,
            },
        ] {
            assert_eq!(Event::parse(&event.line()), Some(event));
        }
    }

    #[test]
    fn a_broken_line_is_no_command_or_event() {
        for line in [
            "",
            "jump",
            "seek",
            "seek -1",
            "seek 5 6",
            "volume 2 0",
            "volume 0.5 yes",
        ] {
            assert_eq!(Command::parse(line), None, "{line:?}");
        }
        for line in [
            "",
            "ready 1 1 1",
            "position -5",
            "frame 1 2",
            "ended now",
            "ready 1 2 0 0",
        ] {
            assert_eq!(Event::parse(line), None, "{line:?}");
        }
        assert_eq!(
            Event::parse(&Event::Failed("two\nlines".to_owned()).line()),
            Some(Event::Failed("twolines".to_owned())),
            "a message stays on its line"
        );
    }

    #[test]
    fn only_a_frame_of_sane_size_is_read() {
        assert_eq!(frame_length(216, 162, 864), Some(864 * 162));
        assert_eq!(frame_length(3, 2, 12), Some(24));
        assert_eq!(frame_length(3, 2, 16), Some(32), "a padded row");
        assert_eq!(frame_length(0, 2, 0), None);
        assert_eq!(frame_length(3, 2, 11), None, "rows shorter than their pixels");
        assert_eq!(frame_length(3, 2, 4096), None, "rows padded beyond reason");
        assert_eq!(
            frame_length(MAX_FRAME_SIDE + 1, 2, (MAX_FRAME_SIDE + 1) * 4),
            None
        );
    }
}
