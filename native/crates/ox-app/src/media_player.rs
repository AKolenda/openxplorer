// SPDX-License-Identifier: AGPL-3.0-only
//! The details pane's inline player (PROP-011) decodes in a process of
//! its own: the app itself, started again with [`PLAYER_FLAG`], as the
//! Details tab's reader is (`media_details.rs`).
//!
//! A recording is untrusted input, and `GStreamer` answers some files, or
//! some moments of their playback, with a failed `g_assert`, which aborts
//! the process it runs in; `decodebin3`'s `mq_slot_handle_stream_start`
//! is one, which GTK 4.22 reaches through `GStreamer` 1.28's `GstPlay`
//! (always `playbin3`). Played in the app, such an abort closed every
//! window, as Windows' Explorer never does for a media file. The player
//! process uses the classic `playbin` instead, and whatever happens to
//! it, the pane only shows the file's icon in place of the player.
//!
//! A player is stopped and dropped (paused, its pipeline set to NULL,
//! then let go) when the selection changes, and before the app renames,
//! moves, deletes or overwrites its file, and no new one starts until the
//! change is done ([`hold_players_within`]).

mod child;
mod protocol;

/// Reads `text` as the player's events and the app's commands, line by
/// line, as each side reads the other, with the frames' lengths, for the
/// fuzz targets.
///
/// # Panics
///
/// When a frame the app would read is larger than the protocol allows.
#[cfg(feature = "fuzzing")]
pub(crate) fn fuzz_protocol(text: &str) {
    for line in text.split_inclusive('\n').take(64) {
        let _ = protocol::Command::parse(line);
        if let Some(protocol::Event::Frame {
            width,
            height,
            stride,
        }) = protocol::Event::parse(line)
        {
            if let Some(length) = protocol::frame_length(width, height, stride) {
                let side = u64::from(protocol::MAX_FRAME_SIDE);
                assert!(length as u64 <= (side * 4 + 63) * side, "{line:?}");
                assert!(length >= (width as usize) * 4 * (height as usize), "{line:?}");
            }
        }
    }
}
mod stream;

#[cfg(test)]
mod tests;

use std::ffi::OsString;

use gtk::glib;

pub(crate) use stream::{hold_players_within, PlayerStream};

/// The first argument that starts the app as a player:
/// `<app> --ox-play-media <audio|video> <max-width> <max-height> <uri>`.
pub(crate) const PLAYER_FLAG: &str = "--ox-play-media";
/// Set, the tests' player process aborts when asked to play.
#[cfg(test)]
pub(crate) const TEST_ABORT_VARIABLE: &str = "OX_TEST_PLAYER_ABORT";
/// Set, the tests' player process ignores Quit and its closed input, as a
/// decoder that hangs does, until it is killed.
#[cfg(test)]
pub(crate) const TEST_STUBBORN_VARIABLE: &str = "OX_TEST_PLAYER_STUBBORN";

/// What a recording has to show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    /// Sound only: no video is decoded, a cover picture included.
    Audio,
    Video,
}

impl Kind {
    const fn argument(self) -> &'static str {
        match self {
            Self::Audio => "audio",
            Self::Video => "video",
        }
    }

    fn from_argument(argument: &str) -> Option<Self> {
        [Self::Audio, Self::Video]
            .into_iter()
            .find(|kind| kind.argument() == argument)
    }
}

/// Runs the app as a player when `arguments` ask for it; the exit status
/// then, `None` for every other command line.
pub(crate) fn run_as_player(arguments: &[String]) -> Option<glib::ExitCode> {
    if arguments.get(1).map(String::as_str) != Some(PLAYER_FLAG) {
        return None;
    }
    let size = |at: usize| arguments.get(at).and_then(|size| size.parse::<u32>().ok());
    let (Some(kind), Some(width), Some(height), Some(uri)) = (
        arguments.get(2).and_then(|kind| Kind::from_argument(kind)),
        size(3),
        size(4),
        arguments.get(5),
    ) else {
        return Some(glib::ExitCode::FAILURE);
    };
    Some(child::run(kind, width.max(1), height.max(1), uri))
}

/// A command line, and the variables it needs in its environment.
type CommandLine = (Vec<OsString>, Vec<(&'static str, String)>);

/// The command line of a player for `uri`.
fn player_command(kind: Kind, max_width: u32, max_height: u32, uri: &str) -> Option<CommandLine> {
    let arguments: [OsString; 5] = [
        PLAYER_FLAG.into(),
        kind.argument().into(),
        max_width.to_string().into(),
        max_height.to_string().into(),
        uri.into(),
    ];
    // The tests' process is not the app: it starts itself again to run
    // only the test that plays (`tests::player_process`), which takes
    // these arguments from the environment.
    if cfg!(test) {
        let program = crate::media_details::reader_program()?;
        let joined = arguments
            .iter()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("\n");
        let command = vec![
            program.into(),
            "media_player::tests::player_process".into(),
            "--exact".into(),
            "--test-threads=1".into(),
        ];
        return Some((command, vec![(TEST_ARGUMENTS_VARIABLE, joined)]));
    }
    let program = crate::media_details::reader_program()?;
    Some((
        std::iter::once(program.into()).chain(arguments).collect(),
        Vec::new(),
    ))
}

/// Where the tests' player process finds its command line.
const TEST_ARGUMENTS_VARIABLE: &str = "OX_TEST_PLAYER_ARGUMENTS";
