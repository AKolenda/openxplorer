// SPDX-License-Identifier: AGPL-3.0-only
//! A video's or audio file's Details, from `GStreamer`: its length, its
//! streams' sizes, rates and channels, and its tags.
//!
//! The discoverer (gst-pbutils) is asked first, as the GNOME file managers
//! ask it. When it finds nothing, as when no decoder is installed for any
//! of the file's streams, the file is only parsed (`urisourcebin !
//! parsebin`): the container still gives the frame size, frame rate,
//! sample rate, channels, length and tags, which Windows shows without a
//! codec too, and no decoder runs on the file. Each gives up after
//! [`READ_LIMIT`]; the process they run in is stopped by the app after a
//! longer time ([`super::READ_TIMEOUT`]).

use std::sync::{Arc, Mutex};

use gstreamer as gst;
use gstreamer::glib;
use gstreamer::prelude::*;
use gstreamer_pbutils as pbutils;
use gstreamer_pbutils::prelude::*;

use super::picture::clip;
use super::MediaDetails;

/// How long the discoverer, then the parser, may take over one file.
const READ_LIMIT: gst::ClockTime = gst::ClockTime::from_seconds(4);
/// How long the parser's late tags are waited for once the file prerolled.
const LATE_TAGS: std::time::Duration = std::time::Duration::from_millis(300);

/// What was read from a file's streams, by either way.
#[derive(Debug, Default)]
pub(super) struct StreamFacts {
    /// Nanoseconds.
    pub length: Option<u64>,
    /// Width, height, frame rate and bit rate of the first video stream.
    pub video: Option<VideoFacts>,
    /// Sample rate, channels and bit rate of the first audio stream.
    pub audio: Option<AudioFacts>,
    /// The file's tags.
    pub tags: Option<gst::TagList>,
}

/// A video stream's facts.
#[derive(Debug, Default)]
pub(super) struct VideoFacts {
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub frame_rate: Option<(u32, u32)>,
    pub bitrate: Option<u32>,
}

/// An audio stream's facts.
#[derive(Debug, Default)]
pub(super) struct AudioFacts {
    pub sample_rate: Option<u32>,
    pub channels: Option<u32>,
    pub bitrate: Option<u32>,
}

/// Reads the streams and tags of the video or audio file at `uri`, of
/// `size` bytes. Nothing is filled in when `GStreamer` cannot read it.
pub(super) fn read_streams(uri: &str, size: Option<u64>, details: &mut MediaDetails) {
    if gst::init().is_err() {
        return;
    }
    let Some(facts) = discover(uri).or_else(|| parse(uri)) else {
        return;
    };
    details.length = facts.length;
    if let Some(video) = &facts.video {
        details.frame_width = video.width;
        details.frame_height = video.height;
        details.frame_rate = video.frame_rate;
        details.data_rate = video.bitrate;
    }
    if let Some(audio) = &facts.audio {
        details.audio_bitrate = audio.bitrate;
        details.channels = audio.channels;
        details.sample_rate = audio.sample_rate;
    }
    if let Some(tags) = &facts.tags {
        apply_tags(tags, details);
    }
    // Windows works out the rates the streams leave out from the file's
    // size and length, as here.
    let average = size.zip(facts.length).and_then(|(size, length)| {
        let bits = u128::from(size) * 8 * 1_000_000_000 / u128::from(length);
        u32::try_from(bits).ok()
    });
    if facts.video.is_some() {
        details.total_bitrate = match (details.data_rate, details.audio_bitrate) {
            (Some(video), Some(audio)) => Some(video.saturating_add(audio)),
            _ => average,
        };
        // The video's share is the rest only when the audio's is known.
        let audio_share = if facts.audio.is_some() {
            details.audio_bitrate
        } else {
            Some(0)
        };
        if details.data_rate.is_none() {
            details.data_rate = details
                .total_bitrate
                .zip(audio_share)
                .map(|(total, audio)| total.saturating_sub(audio));
        }
    } else if details.audio_bitrate.is_none() && facts.audio.is_some() {
        details.audio_bitrate = average;
    }
    details.rating_readable = true;
}

/// What the discoverer found in the file at `uri`; `None` when it found
/// no stream. Its asynchronous form is used, as it also hands over what it
/// read before it failed, as when a decoder is missing for one stream.
fn discover(uri: &str) -> Option<StreamFacts> {
    let context = glib::MainContext::new();
    let info = context
        .with_thread_default(|| {
            let discoverer = pbutils::Discoverer::new(READ_LIMIT).ok()?;
            let found = Arc::new(Mutex::new(None));
            let kept = Arc::clone(&found);
            discoverer.connect_discovered(move |_, info, _| {
                let has_streams = !info.audio_streams().is_empty() || !info.video_streams().is_empty();
                if has_streams {
                    if let Ok(mut found) = kept.lock() {
                        *found = Some(info.clone());
                    }
                }
            });
            let main_loop = glib::MainLoop::new(Some(&context), false);
            let finished = main_loop.clone();
            discoverer.connect_finished(move |_| finished.quit());
            discoverer.start();
            if discoverer.discover_uri_async(uri).is_err() {
                discoverer.stop();
                return None;
            }
            main_loop.run();
            discoverer.stop();
            let info = found.lock().ok()?.take();
            info
        })
        .ok()
        .flatten()?;
    let video = info
        .video_streams()
        .into_iter()
        .find(|video| !video.is_image())
        .map(|video| {
            let rate = video.framerate();
            VideoFacts {
                width: Some(video.width()).filter(|width| *width > 0),
                height: Some(video.height()).filter(|height| *height > 0),
                frame_rate: fraction(rate.numer(), rate.denom()),
                bitrate: known_rate(video.bitrate(), video.max_bitrate()),
            }
        });
    let audio = info.audio_streams().into_iter().next().map(|audio| AudioFacts {
        sample_rate: Some(audio.sample_rate()).filter(|rate| *rate > 0),
        channels: Some(audio.channels()).filter(|channels| *channels > 0),
        bitrate: known_rate(audio.bitrate(), audio.max_bitrate())
            .or_else(|| audio.tags().and_then(|tags| tag_bitrate(&tags))),
    });
    Some(StreamFacts {
        length: info
            .duration()
            .map(gst::ClockTime::nseconds)
            .filter(|length| *length > 0),
        video,
        audio,
        tags: info.tags(),
    })
}

/// What parsing the file at `uri` without decoding it gives: the caps and
/// tags of its parsed streams and its length; `None` without a stream.
pub(super) fn parse(uri: &str) -> Option<StreamFacts> {
    let source = gst::ElementFactory::make("urisourcebin")
        .property("uri", uri)
        .build()
        .ok()?;
    let parser = gst::ElementFactory::make("parsebin").build().ok()?;
    let pipeline = gst::Pipeline::new();
    pipeline.add_many([&source, &parser]).ok()?;
    let parser_input = parser.downgrade();
    source.connect_pad_added(move |_, pad| {
        if let Some(sink) = parser_input
            .upgrade()
            .and_then(|parser| parser.static_pad("sink"))
        {
            if !sink.is_linked() {
                let _ = pad.link(&sink);
            }
        }
    });
    let streams = Arc::new(Mutex::new(Vec::new()));
    let found = Arc::clone(&streams);
    let bin = pipeline.downgrade();
    parser.connect_pad_added(move |_, pad| {
        let Some(pipeline) = bin.upgrade() else {
            return;
        };
        if let Some(caps) = pad.current_caps() {
            if let Ok(mut found) = found.lock() {
                found.push(caps);
            }
        }
        // Each stream ends in a sink that drops it, so the file prerolls.
        let Ok(sink) = gst::ElementFactory::make("fakesink")
            .property("sync", false)
            .build()
        else {
            return;
        };
        if pipeline.add(&sink).is_ok() {
            let _ = sink.sync_state_with_parent();
            if let Some(input) = sink.static_pad("sink") {
                let _ = pad.link(&input);
            }
        }
    });
    let mut tags = gst::TagList::new();
    // A stream no parser takes leaves the file unprerolled, yet the
    // container already told its length.
    preroll(&pipeline, &mut tags);
    let length = pipeline
        .query_duration::<gst::ClockTime>()
        .map(gst::ClockTime::nseconds)
        .filter(|length| *length > 0);
    let _ = pipeline.set_state(gst::State::Null);
    let caps = streams.lock().ok()?.clone();
    let mut facts = StreamFacts {
        length,
        tags: (tags.n_tags() > 0).then_some(tags),
        ..StreamFacts::default()
    };
    for caps in &caps {
        let Some(structure) = caps.structure(0) else {
            continue;
        };
        let positive = |field: &str| {
            structure
                .get::<i32>(field)
                .ok()
                .and_then(|value| u32::try_from(value).ok())
                .filter(|value| *value > 0)
        };
        if structure.name().starts_with("video/") && facts.video.is_none() {
            let rate = structure.get::<gst::Fraction>("framerate").ok();
            facts.video = Some(VideoFacts {
                width: positive("width"),
                height: positive("height"),
                frame_rate: rate.and_then(|rate| fraction(rate.numer(), rate.denom())),
                bitrate: None,
            });
        } else if structure.name().starts_with("audio/") && facts.audio.is_none() {
            facts.audio = Some(AudioFacts {
                sample_rate: positive("rate"),
                channels: positive("channels"),
                bitrate: None,
            });
        }
    }
    (facts.video.is_some() || facts.audio.is_some()).then_some(facts)
}

/// Pauses `pipeline` until it has prerolled, failed or taken
/// [`READ_LIMIT`], gathering the tags it posts into `tags`.
fn preroll(pipeline: &gst::Pipeline, tags: &mut gst::TagList) {
    let Some(bus) = pipeline.bus() else {
        return;
    };
    if pipeline.set_state(gst::State::Paused).is_err() {
        return;
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_nanos(READ_LIMIT.nseconds());
    loop {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        if left.is_zero() {
            return;
        }
        let wait = gst::ClockTime::from_nseconds(u64::try_from(left.as_nanos()).unwrap_or(u64::MAX));
        let Some(message) = bus.timed_pop(wait) else {
            return;
        };
        match message.view() {
            gst::MessageView::Tag(tag) => {
                tags.make_mut().insert(&tag.tags(), gst::TagMergeMode::Keep);
            }
            // A parser may send its tags just after the first buffers.
            gst::MessageView::AsyncDone(_) => return gather_late_tags(&bus, tags),
            gst::MessageView::Error(_) | gst::MessageView::Eos(_) => return,
            _ => {}
        }
    }
}

/// Gathers the tags posted within [`LATE_TAGS`] after the file prerolled.
fn gather_late_tags(bus: &gst::Bus, tags: &mut gst::TagList) {
    let deadline = std::time::Instant::now() + LATE_TAGS;
    while let Some(left) = deadline
        .checked_duration_since(std::time::Instant::now())
        .filter(|left| !left.is_zero())
    {
        let wait = gst::ClockTime::from_nseconds(u64::try_from(left.as_nanos()).unwrap_or(u64::MAX));
        let Some(message) = bus.timed_pop_filtered(wait, &[gst::MessageType::Tag]) else {
            return;
        };
        if let gst::MessageView::Tag(tag) = message.view() {
            tags.make_mut().insert(&tag.tags(), gst::TagMergeMode::Keep);
        }
    }
}

/// `numerator / denominator` when both are positive.
fn fraction(numerator: i32, denominator: i32) -> Option<(u32, u32)> {
    let numerator = u32::try_from(numerator).ok().filter(|value| *value > 0)?;
    let denominator = u32::try_from(denominator).ok().filter(|value| *value > 0)?;
    Some((numerator, denominator))
}

/// A stream's bit rate, or its largest when only that is known.
fn known_rate(bitrate: u32, max_bitrate: u32) -> Option<u32> {
    Some(bitrate)
        .filter(|rate| *rate > 0)
        .or_else(|| Some(max_bitrate).filter(|rate| *rate > 0))
}

fn tag_bitrate(tags: &gst::TagList) -> Option<u32> {
    tags.get::<gst::tags::Bitrate>()
        .map(|value| value.get())
        .or_else(|| tags.get::<gst::tags::NominalBitrate>().map(|value| value.get()))
        .filter(|rate| *rate > 0)
}

/// The tags Windows' Details tab shows, under its names.
fn apply_tags(tags: &gst::TagList, details: &mut MediaDetails) {
    let text = |values: Vec<String>| {
        let joined = values
            .into_iter()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>()
            .join("; ");
        (!joined.is_empty()).then(|| clip(&joined))
    };
    macro_rules! all {
        ($tag:ty) => {
            text(
                (0..tags.size::<$tag>())
                    .filter_map(|index| {
                        tags.index::<$tag>(index)
                            .map(|value| value.get().to_owned())
                    })
                    .collect(),
            )
        };
    }
    details.title = all!(gst::tags::Title);
    details.comments = all!(gst::tags::Comment).or_else(|| all!(gst::tags::Description));
    details.artists = all!(gst::tags::Artist);
    details.album_artist = all!(gst::tags::AlbumArtist);
    details.album = all!(gst::tags::Album);
    details.genre = all!(gst::tags::Genre);
    details.composers = all!(gst::tags::Composer);
    details.conductors = all!(gst::tags::Conductor);
    details.publisher = all!(gst::tags::Publisher);
    details.encoded_by = all!(gst::tags::EncodedBy);
    details.copyright = all!(gst::tags::Copyright);
    details.tags = all!(gst::tags::Keywords);
    details.track = tags
        .get::<gst::tags::TrackNumber>()
        .map(|value| value.get())
        .filter(|track| *track > 0);
    details.year = tags
        .get::<gst::tags::DateTime>()
        .and_then(|value| u32::try_from(value.get().year()).ok())
        .or_else(|| {
            tags.get::<gst::tags::Date>()
                .map(|value| u32::from(value.get().year()))
        })
        .filter(|year| *year > 0);
    // GStreamer rates from 0 to 100. Windows writes its five stars as 1,
    // 25, 50, 75 and 99, and reads the ranges around them.
    details.rating = tags
        .get::<gst::tags::UserRating>()
        .map(|value| match value.get() {
            0 => 0,
            1..=12 => 1,
            13..=37 => 2,
            38..=62 => 3,
            63..=87 => 4,
            _ => 5,
        });
    if details.audio_bitrate.is_none() && details.frame_width.is_none() {
        details.audio_bitrate = tag_bitrate(tags);
    }
}
