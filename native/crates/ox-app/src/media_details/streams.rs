// SPDX-License-Identifier: AGPL-3.0-only
//! A video's or audio file's Details, from GStreamer's discoverer
//! (gst-pbutils): its length, its streams' sizes, rates and channels, and
//! its tags. The discoverer gives up after [`DISCOVER_TIMEOUT`]; the
//! process it runs in is stopped by the app after a longer one
//! ([`super::READ_TIMEOUT`]).

use gstreamer as gst;
use gstreamer_pbutils as pbutils;
use gstreamer_pbutils::prelude::*;

use super::picture::clip;
use super::MediaDetails;

/// How long the discoverer may take over one file.
const DISCOVER_TIMEOUT: gst::ClockTime = gst::ClockTime::from_seconds(5);

/// Reads the streams and tags of the video or audio file at `uri`.
/// Nothing is filled in when GStreamer cannot read it.
pub(super) fn read_streams(uri: &str, size: Option<u64>, details: &mut MediaDetails) {
    if gst::init().is_err() {
        return;
    }
    let Ok(discoverer) = pbutils::Discoverer::new(DISCOVER_TIMEOUT) else {
        return;
    };
    let Ok(info) = discoverer.discover_uri(uri) else {
        return;
    };
    let length = info
        .duration()
        .map(gst::ClockTime::nseconds)
        .filter(|length| *length > 0);
    details.length = length;
    // A missing plug-in still gives what the container said.
    if let Some(video) = info.video_streams().into_iter().find(|video| !video.is_image()) {
        details.frame_width = Some(video.width()).filter(|width| *width > 0);
        details.frame_height = Some(video.height()).filter(|height| *height > 0);
        let rate = video.framerate();
        if rate.numer() > 0 && rate.denom() > 0 {
            details.frame_rate = Some((rate.numer().unsigned_abs(), rate.denom().unsigned_abs()));
        }
        details.data_rate = known_rate(video.bitrate(), video.max_bitrate());
    }
    if let Some(audio) = info.audio_streams().into_iter().next() {
        details.audio_bitrate = known_rate(audio.bitrate(), audio.max_bitrate());
        details.channels = Some(audio.channels()).filter(|channels| *channels > 0);
        details.sample_rate = Some(audio.sample_rate()).filter(|rate| *rate > 0);
        if details.audio_bitrate.is_none() {
            details.audio_bitrate = audio.tags().and_then(|tags| tag_bitrate(&tags));
        }
    }
    if let Some(tags) = info.tags() {
        apply_tags(&tags, details);
    }
    // Windows works out the rates the streams leave out from the file's
    // size and length, as here.
    let average = size.zip(length).and_then(|(size, length)| {
        let bits = u128::from(size) * 8 * 1_000_000_000 / u128::from(length);
        u32::try_from(bits).ok()
    });
    let is_video = details.frame_width.is_some();
    if is_video {
        details.total_bitrate = match (details.data_rate, details.audio_bitrate) {
            (Some(video), Some(audio)) => Some(video.saturating_add(audio)),
            _ => average,
        };
        if details.data_rate.is_none() {
            details.data_rate = details
                .total_bitrate
                .map(|total| total.saturating_sub(details.audio_bitrate.unwrap_or(0)));
        }
    } else if details.audio_bitrate.is_none() && details.channels.is_some() {
        details.audio_bitrate = average;
    }
    details.rating_readable = true;
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
    if details.audio_bitrate.is_none() {
        details.audio_bitrate = tag_bitrate(tags).filter(|_| details.frame_width.is_none());
    }
}
