// SPDX-License-Identifier: AGPL-3.0-only
//! The Details reader over pictures, videos and audio files made here,
//! with fictional tags, and over damaged ones.

use std::ffi::OsStr;
use std::path::Path;
use std::time::{Duration, Instant};

use exif::experimental::Writer;
use exif::{Field, In, Rational, SRational, Tag, Value};
use gstreamer as gst;
use gstreamer::prelude::*;
use gtk::gio;
use gtk::glib;

use super::*;

/// A JPEG of `width` by `height` pixels, with `exif` (a TIFF structure)
/// in an APP1 segment after its start, as cameras write it.
pub(crate) fn jpeg_with_exif(width: i32, height: i32, exif: &[u8]) -> Vec<u8> {
    let pixels = gtk::gdk_pixbuf::Pixbuf::new(gtk::gdk_pixbuf::Colorspace::Rgb, false, 8, width, height)
        .expect("pixels");
    pixels.fill(0x3a6e_a5ff);
    let jpeg = pixels.save_to_bufferv("jpeg", &[]).expect("a JPEG");
    let mut segment = b"Exif\0\0".to_vec();
    segment.extend_from_slice(exif);
    let length = u16::try_from(segment.len() + 2).expect("a short segment");
    let mut out = jpeg[..2].to_vec();
    out.extend_from_slice(&[0xff, 0xe1]);
    out.extend_from_slice(&length.to_be_bytes());
    out.extend_from_slice(&segment);
    out.extend_from_slice(&jpeg[2..]);
    out
}

/// The EXIF of a fictional camera's photo, with a Windows title and four
/// stars.
pub(crate) fn camera_exif() -> Vec<u8> {
    let ascii = |text: &str| Value::Ascii(vec![text.as_bytes().to_vec()]);
    let rational = |num, denom| Value::Rational(vec![Rational { num, denom }]);
    let mut title: Vec<u8> = "Harbour at dusk"
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
    title.extend_from_slice(&[0, 0]);
    let fields = [
        Field {
            tag: Tag::Make,
            ifd_num: In::PRIMARY,
            value: ascii("Fictional Optics"),
        },
        Field {
            tag: Tag::Model,
            ifd_num: In::PRIMARY,
            value: ascii("FO-100"),
        },
        Field {
            tag: Tag::Software,
            ifd_num: In::PRIMARY,
            value: ascii("Example Editor 2"),
        },
        Field {
            tag: Tag::Artist,
            ifd_num: In::PRIMARY,
            value: ascii("A. Example"),
        },
        Field {
            tag: Tag::XResolution,
            ifd_num: In::PRIMARY,
            value: rational(300, 1),
        },
        Field {
            tag: Tag::YResolution,
            ifd_num: In::PRIMARY,
            value: rational(300, 1),
        },
        Field {
            tag: Tag::ResolutionUnit,
            ifd_num: In::PRIMARY,
            value: Value::Short(vec![2]),
        },
        Field {
            tag: Tag(exif::Context::Tiff, 0x9c9b),
            ifd_num: In::PRIMARY,
            value: Value::Byte(title),
        },
        Field {
            tag: Tag(exif::Context::Tiff, 0x4746),
            ifd_num: In::PRIMARY,
            value: Value::Short(vec![4]),
        },
        Field {
            tag: Tag::DateTimeOriginal,
            ifd_num: In::PRIMARY,
            value: ascii("2026:05:01 18:42:07"),
        },
        Field {
            tag: Tag::FNumber,
            ifd_num: In::PRIMARY,
            value: rational(28, 10),
        },
        Field {
            tag: Tag::ExposureTime,
            ifd_num: In::PRIMARY,
            value: rational(1, 125),
        },
        Field {
            tag: Tag::PhotographicSensitivity,
            ifd_num: In::PRIMARY,
            value: Value::Short(vec![200]),
        },
        Field {
            tag: Tag::ExposureBiasValue,
            ifd_num: In::PRIMARY,
            value: Value::SRational(vec![SRational { num: -2, denom: 3 }]),
        },
        Field {
            tag: Tag::FocalLength,
            ifd_num: In::PRIMARY,
            value: rational(50, 1),
        },
        Field {
            tag: Tag::MeteringMode,
            ifd_num: In::PRIMARY,
            value: Value::Short(vec![5]),
        },
        Field {
            tag: Tag::Flash,
            ifd_num: In::PRIMARY,
            value: Value::Short(vec![0x10]),
        },
        Field {
            tag: Tag::FocalLengthIn35mmFilm,
            ifd_num: In::PRIMARY,
            value: Value::Short(vec![75]),
        },
        Field {
            tag: Tag::ColorSpace,
            ifd_num: In::PRIMARY,
            value: Value::Short(vec![1]),
        },
        Field {
            tag: Tag::LensModel,
            ifd_num: In::PRIMARY,
            value: ascii("FO 50mm F2.8"),
        },
        Field {
            tag: Tag::ExposureProgram,
            ifd_num: In::PRIMARY,
            value: Value::Short(vec![3]),
        },
        Field {
            tag: Tag::WhiteBalance,
            ifd_num: In::PRIMARY,
            value: Value::Short(vec![0]),
        },
        Field {
            tag: Tag::ExifVersion,
            ifd_num: In::PRIMARY,
            value: Value::Undefined(b"0232".to_vec(), 0),
        },
    ];
    let mut writer = Writer::new();
    for field in &fields {
        writer.push_field(field);
    }
    let mut tiff = std::io::Cursor::new(Vec::new());
    writer.write(&mut tiff, true).expect("the EXIF");
    tiff.into_inner()
}

/// Encodes `pipeline`, a gst-launch description ending in
/// `filesink location=<path>`, tagging the element named `tagged` with
/// `tags`, and waits for it to finish.
pub(crate) fn encode(pipeline: &str, tagged: &str, tags: &gst::TagList) {
    gst::init().expect("GStreamer");
    let pipeline = gst::parse::launch(pipeline)
        .expect("the pipeline")
        .downcast::<gst::Pipeline>()
        .expect("a pipeline");
    let setter = pipeline
        .by_name(tagged)
        .and_then(|element| element.dynamic_cast::<gst::TagSetter>().ok())
        .expect("an encoder that takes tags");
    setter.merge_tags(tags, gst::TagMergeMode::Replace);
    pipeline.set_state(gst::State::Playing).expect("playing");
    let bus = pipeline.bus().expect("a bus");
    let message = bus
        .timed_pop_filtered(
            gst::ClockTime::from_seconds(60),
            &[gst::MessageType::Eos, gst::MessageType::Error],
        )
        .expect("the encoder finished");
    pipeline.set_state(gst::State::Null).expect("stopped");
    assert_eq!(message.type_(), gst::MessageType::Eos, "{message:?}");
}

/// A fictional song's tags.
pub(crate) fn song_tags() -> gst::TagList {
    gst::init().expect("GStreamer");
    let mut tags = gst::TagList::new();
    {
        let tags = tags.get_mut().expect("a new list");
        tags.add::<gst::tags::Title>(&"Lighthouse Waltz", gst::TagMergeMode::Append);
        tags.add::<gst::tags::Artist>(&"The Example Quartet", gst::TagMergeMode::Append);
        tags.add::<gst::tags::AlbumArtist>(&"Example Ensemble", gst::TagMergeMode::Append);
        tags.add::<gst::tags::Album>(&"Coastal Sketches", gst::TagMergeMode::Append);
        tags.add::<gst::tags::TrackNumber>(&3, gst::TagMergeMode::Append);
        tags.add::<gst::tags::Genre>(&"Folk", gst::TagMergeMode::Append);
        tags.add::<gst::tags::Composer>(&"B. Placeholder", gst::TagMergeMode::Append);
        let date = gst::DateTime::from_ymd(2025, 6, 1).expect("a date");
        tags.add::<gst::tags::DateTime>(&date, gst::TagMergeMode::Append);
    }
    tags
}

/// A two-second Ogg Vorbis song at 44.1 kHz in stereo, with
/// [`song_tags`].
pub(crate) fn write_song(path: &Path) {
    let pipeline = format!(
        "audiotestsrc num-buffers=86 samplesperbuffer=1024 ! audio/x-raw,rate=44100,channels=2 ! \
         audioconvert ! vorbisenc name=encoder ! oggmux ! filesink location=\"{}\"",
        path.display()
    );
    encode(&pipeline, "encoder", &song_tags());
}

/// A two-second Ogg Theora video of 320 by 240 at 15 frames a second,
/// with a mono 48 kHz Vorbis track and a title.
pub(crate) fn write_video(path: &Path) {
    let pipeline = format!(
        "videotestsrc num-buffers=30 ! video/x-raw,width=320,height=240,framerate=15/1 ! \
         videoconvert ! theoraenc ! oggmux name=mux ! filesink location=\"{}\" \
         audiotestsrc num-buffers=94 samplesperbuffer=1024 ! audio/x-raw,rate=48000,channels=1 ! \
         audioconvert ! vorbisenc name=encoder ! mux.",
        path.display()
    );
    gst::init().expect("GStreamer");
    let mut tags = gst::TagList::new();
    tags.get_mut()
        .expect("a new list")
        .add::<gst::tags::Title>(&"Harbour walk", gst::TagMergeMode::Append);
    encode(&pipeline, "encoder", &tags);
}

fn uri_of(path: &Path) -> String {
    gio::File::for_path(path).uri().to_string()
}

/// A photo's EXIF gives Windows' Description, Origin, Image, Camera and
/// Advanced photo values.
///
/// parity: PROP-034
#[test]
fn a_photos_exif_is_read_from_its_start() {
    let folder = tempfile::tempdir().expect("a folder");
    let photo = folder.path().join("Harbour.jpg");
    std::fs::write(&photo, jpeg_with_exif(64, 48, &camera_exif())).expect("the photo");
    let size = std::fs::metadata(&photo).expect("the photo").len();

    let details = read_details(&uri_of(&photo), MediaKind::Picture, Some(size));

    assert_eq!((details.width, details.height), (Some(64), Some(48)));
    assert_eq!(details.bit_depth, Some(24));
    assert_eq!(details.resolution, Some((300.0, 300.0)));
    assert_eq!(details.title.as_deref(), Some("Harbour at dusk"));
    assert_eq!(details.rating, Some(4));
    assert_eq!(details.authors.as_deref(), Some("A. Example"));
    assert_eq!(details.program_name.as_deref(), Some("Example Editor 2"));
    assert_eq!(
        details.date_taken,
        Some(NaiveDateTime {
            year: 2026,
            month: 5,
            day: 1,
            hour: 18,
            minute: 42,
            second: 7
        })
    );
    assert_eq!(details.camera_maker.as_deref(), Some("Fictional Optics"));
    assert_eq!(details.camera_model.as_deref(), Some("FO-100"));
    assert_eq!(details.f_number, Some(2.8));
    assert_eq!(details.exposure_time, Some((1, 125)));
    assert_eq!(details.iso, Some(200));
    assert!(details
        .exposure_bias
        .is_some_and(|bias| (bias + 0.667).abs() < 0.01));
    assert_eq!(details.focal_length, Some(50.0));
    assert_eq!(details.metering_mode, Some(5));
    assert_eq!(details.flash, Some(0x10));
    assert_eq!(details.focal_length_35mm, Some(75));
    assert_eq!(details.color_space, Some(1));
    assert_eq!(details.lens_model.as_deref(), Some("FO 50mm F2.8"));
    assert_eq!(details.exposure_program, Some(3));
    assert_eq!(details.white_balance, Some(0));
    assert_eq!(details.exif_version.as_deref(), Some("0232"));
}

/// A PNG gives its size, bit depth, resolution and text chunks, and an
/// unrated picture still has a rating to show.
///
/// parity: PROP-034
#[test]
fn a_png_gives_its_header_and_text() {
    let folder = tempfile::tempdir().expect("a folder");
    let picture = folder.path().join("Screenshot.png");
    let pixels =
        gtk::gdk_pixbuf::Pixbuf::new(gtk::gdk_pixbuf::Colorspace::Rgb, true, 8, 485, 596).expect("pixels");
    pixels.fill(0xffff_ffff);
    pixels
        .savev(
            &picture,
            "png",
            &[
                ("tEXt::Title", "Floor plan"),
                ("tEXt::Software", "Example Capture"),
                ("x-dpi", "96"),
                ("y-dpi", "96"),
            ],
        )
        .expect("the picture");

    let details = read_details(&uri_of(&picture), MediaKind::Picture, None);

    assert_eq!((details.width, details.height), (Some(485), Some(596)));
    assert_eq!(details.bit_depth, Some(32));
    let (horizontal, vertical) = details.resolution.expect("a resolution");
    assert_eq!((horizontal.round(), vertical.round()), (96.0, 96.0));
    assert_eq!(details.title.as_deref(), Some("Floor plan"));
    assert_eq!(details.program_name.as_deref(), Some("Example Capture"));
    assert_eq!(details.rating, None);
    assert!(details.rating_readable, "an unrated PNG shows empty stars");
}

/// Damaged and hostile pictures (cut short, lengths past their end, a
/// marker loop, random bytes) give what they can and never panic.
///
/// parity: PROP-034
#[test]
fn damaged_pictures_are_read_safely() {
    let photo = jpeg_with_exif(16, 16, &camera_exif());
    let mut cases: Vec<Vec<u8>> = (0..photo.len())
        .step_by(7)
        .map(|end| photo[..end].to_vec())
        .collect();
    cases.push(b"\x89PNG\r\n\x1a\n\xff\xff\xff\xffIHDR".to_vec());
    cases.push([&[0xff, 0xd8][..], &[0xff; 4096][..]].concat());
    cases.push(
        [
            &[0xff, 0xd8, 0xff, 0xe1, 0xff, 0xff][..],
            b"Exif\0\0II*\0\xff\xff\xff\xff",
        ]
        .concat(),
    );
    cases.push(
        b"BM"
            .iter()
            .copied()
            .chain(std::iter::repeat_n(0xff, 60))
            .collect(),
    );
    let mut state: u32 = 0x1234_5678;
    for _ in 0..64 {
        let noise: Vec<u8> = (0..512)
            .map(|_| {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                state.to_be_bytes()[0]
            })
            .collect();
        cases.push([&[0xff, 0xd8][..], &noise[..]].concat());
        cases.push([&b"\x89PNG\r\n\x1a\n"[..], &noise[..]].concat());
    }
    for case in cases {
        let mut details = MediaDetails::default();
        picture::read_picture(&case, &mut details);
    }
    let mut details = MediaDetails::default();
    picture::read_picture(&photo[..photo.len() / 2], &mut details);
    assert_eq!(
        details.camera_model.as_deref(),
        Some("FO-100"),
        "the EXIF comes first"
    );
}

/// An audio file's tags, length, rate and channels come from GStreamer's
/// discoverer.
///
/// parity: PROP-034
#[test]
fn a_songs_tags_and_stream_are_discovered() {
    let folder = tempfile::tempdir().expect("a folder");
    let song = folder.path().join("Lighthouse Waltz.ogg");
    write_song(&song);
    let size = std::fs::metadata(&song).expect("the song").len();

    let details = read_details(&uri_of(&song), MediaKind::Audio, Some(size));

    assert_eq!(details.title.as_deref(), Some("Lighthouse Waltz"));
    assert_eq!(details.artists.as_deref(), Some("The Example Quartet"));
    assert_eq!(details.album_artist.as_deref(), Some("Example Ensemble"));
    assert_eq!(details.album.as_deref(), Some("Coastal Sketches"));
    assert_eq!(details.track, Some(3));
    assert_eq!(details.genre.as_deref(), Some("Folk"));
    assert_eq!(details.composers.as_deref(), Some("B. Placeholder"));
    assert_eq!(details.year, Some(2025));
    assert_eq!(details.channels, Some(2));
    assert_eq!(details.sample_rate, Some(44_100));
    assert!(details.audio_bitrate.is_some_and(|rate| rate > 0));
    let length = details.length.expect("a length");
    assert!((1_900_000_000..2_200_000_000).contains(&length), "{length}");
    assert!(details.rating_readable);
}

/// A video's frame size, frame rate, length, audio and title come from
/// GStreamer's discoverer, and the rates it leaves out from the size.
///
/// parity: PROP-034
#[test]
fn a_videos_streams_are_discovered() {
    let folder = tempfile::tempdir().expect("a folder");
    let video = folder.path().join("Harbour walk.ogv");
    write_video(&video);
    let size = std::fs::metadata(&video).expect("the video").len();

    let details = read_details(&uri_of(&video), MediaKind::Video, Some(size));

    assert_eq!(details.title.as_deref(), Some("Harbour walk"));
    assert_eq!(
        (details.frame_width, details.frame_height),
        (Some(320), Some(240))
    );
    assert_eq!(details.frame_rate, Some((15, 1)));
    assert_eq!(details.channels, Some(1));
    assert_eq!(details.sample_rate, Some(48_000));
    assert!(details.length.is_some_and(|length| length > 1_500_000_000));
    assert!(details.total_bitrate.is_some_and(|rate| rate > 0));
    assert!(details.data_rate.is_some_and(|rate| rate > 0));
}

/// A file GStreamer cannot read, or that is not there, gives no rows.
///
/// parity: PROP-034
#[test]
fn an_unreadable_media_file_gives_nothing() {
    let folder = tempfile::tempdir().expect("a folder");
    let broken = folder.path().join("broken.ogg");
    std::fs::write(&broken, b"OggS not really").expect("a file");
    let missing = folder.path().join("missing.ogg");

    for path in [broken, missing] {
        let details = read_details(&uri_of(&path), MediaKind::Audio, Some(15));
        assert_eq!(details.title, None);
        assert_eq!(details.length, None);
    }
    let details = read_details(
        &uri_of(&folder.path().join("missing.jpg")),
        MediaKind::Picture,
        None,
    );
    assert_eq!(details, MediaDetails::default());
}

/// What the reader writes reads back the same.
#[test]
fn details_cross_the_process_as_json() {
    let details = MediaDetails {
        title: Some("Lighthouse Waltz".to_owned()),
        frame_rate: Some((30_000, 1001)),
        resolution: Some((96.0, 72.0)),
        date_taken: NaiveDateTime::parse_exif("2026:05:01 18:42:07"),
        ..MediaDetails::default()
    };
    let json = serde_json::to_string(&details).expect("JSON");
    assert_eq!(
        serde_json::from_str::<MediaDetails>(&json).expect("the details"),
        details
    );
}

/// EXIF dates read only when well formed.
#[test]
fn exif_dates_are_checked() {
    assert!(NaiveDateTime::parse_exif("2026:05:01 18:42:07").is_some());
    for text in [
        "",
        "0000:00:00 00:00:00",
        "2026:13:01 18:42:07",
        "2026-05-01 18:42:07",
        "2026:05:01 25:00:00",
    ] {
        assert_eq!(NaiveDateTime::parse_exif(text), None, "{text}");
    }
}

/// A reader that takes too long is stopped, and the tab waits no longer.
///
/// parity: PROP-034
#[gtk::test]
fn a_slow_reader_is_stopped_after_its_time() {
    let process = ReaderProcess::default();
    let started = Instant::now();
    let arguments = [OsStr::new("sleep"), OsStr::new("30")];
    let output = glib::MainContext::default().block_on(run_with_timeout(
        &arguments,
        Duration::from_millis(300),
        &process,
    ));
    assert_eq!(output, None);
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "the reader was stopped"
    );
    assert!(process.0.borrow().is_none(), "no process is kept");

    let arguments = [OsStr::new("echo"), OsStr::new("{}")];
    let output = glib::MainContext::default().block_on(run_with_timeout(
        &arguments,
        Duration::from_secs(10),
        &process,
    ));
    assert_eq!(output.as_deref(), Some(&b"{}\n"[..]));
}

/// Closing the window stops a reader still running.
///
/// parity: PROP-034
#[gtk::test]
fn stopping_ends_a_running_reader() {
    let process = ReaderProcess::default();
    let started = Instant::now();
    let reading = process.clone();
    let stop = glib::timeout_add_local_once(Duration::from_millis(200), move || reading.stop());
    let arguments = [OsStr::new("sleep"), OsStr::new("30")];
    let output = glib::MainContext::default().block_on(run_with_timeout(
        &arguments,
        Duration::from_secs(20),
        &process,
    ));
    drop(stop);
    assert_eq!(output, None);
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "the reader was stopped"
    );
}

/// The reader process runs only for its own first argument.
#[test]
fn only_the_reader_flag_starts_the_reader() {
    assert!(run_as_reader(&["openxplorer".to_owned()]).is_none());
    assert!(run_as_reader(&["openxplorer".to_owned(), "--new-window".to_owned()]).is_none());
    let bad = run_as_reader(&[
        "openxplorer".to_owned(),
        READER_FLAG.to_owned(),
        "nothing".to_owned(),
    ]);
    assert_eq!(bad, Some(glib::ExitCode::FAILURE));
}
