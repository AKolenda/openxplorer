// SPDX-License-Identifier: AGPL-3.0-only
//! The rows of the Details tab, under Windows 11's headings, in its order
//! and with its wording (PROP-034).
//!
//! A picture's rows are Description, Origin, Image, Camera and Advanced
//! photo; a video's Description, Video, Audio, Media, Origin and Content;
//! an audio file's Description, Media, Audio, Origin and Content. Every
//! file ends with File. A row Linux has no source for, or that the file
//! does not have, is left out, and so is a heading left without rows;
//! Rating is shown as five stars wherever the format keeps a rating.

use gtk::gio;
use gtk::prelude::*;
use ox_core::format;
use ox_core::location::LocationContext;

use super::metadata::ItemProperties;
use crate::i18n::message_id;
use crate::media_details::{MediaDetails, MediaKind};

/// A row's value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum DetailValue {
    /// Text the user can select and copy.
    Text(String),
    /// Stars out of five.
    Stars(u8),
}

impl DetailValue {
    /// The value as text: what Ctrl+C copies and a screen reader reads.
    pub(super) fn text(&self) -> String {
        match self {
            DetailValue::Text(text) => text.clone(),
            DetailValue::Stars(0) => ox_core::i18n::gettext("Unrated"),
            DetailValue::Stars(stars) => ox_core::i18n::format_message(
                &ox_core::i18n::ngettext("{count} star", "{count} stars", u64::from(*stars)),
                &[("count", &stars.to_string())],
            ),
        }
    }
}

/// One row: the property's name and its value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DetailRow {
    pub name: String,
    pub value: DetailValue,
}

/// A heading and its rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DetailSection {
    pub heading: String,
    pub rows: Vec<DetailRow>,
}

/// Collects one heading's rows, leaving out those without a value.
struct Section {
    heading: String,
    rows: Vec<DetailRow>,
}

impl Section {
    fn new(heading: &str) -> Self {
        Self {
            heading: ox_core::i18n::gettext(heading),
            rows: Vec::new(),
        }
    }

    fn text(&mut self, name: &str, value: Option<String>) -> &mut Self {
        if let Some(value) = value.filter(|value| !value.trim().is_empty()) {
            self.rows.push(DetailRow {
                name: ox_core::i18n::gettext(name),
                value: DetailValue::Text(value),
            });
        }
        self
    }

    fn stars(&mut self, name: &str, stars: Option<u8>) -> &mut Self {
        if let Some(stars) = stars {
            self.rows.push(DetailRow {
                name: ox_core::i18n::gettext(name),
                value: DetailValue::Stars(stars.min(5)),
            });
        }
        self
    }

    fn finish(self, sections: &mut Vec<DetailSection>) {
        if !self.rows.is_empty() {
            sections.push(DetailSection {
                heading: self.heading,
                rows: self.rows,
            });
        }
    }
}

/// The Details of `properties`, with `media` when the file is a picture,
/// a video or an audio file and its reader finished.
pub(super) fn detail_sections(
    properties: &ItemProperties,
    kind: Option<MediaKind>,
    media: Option<&MediaDetails>,
    locations: &LocationContext,
) -> Vec<DetailSection> {
    let empty = MediaDetails::default();
    let media = media.unwrap_or(&empty);
    let mut sections = Vec::new();
    match kind {
        Some(MediaKind::Picture) => picture_sections(properties.dimensions, media, &mut sections),
        Some(MediaKind::Video) => video_sections(media, &mut sections),
        Some(MediaKind::Audio) => audio_sections(media, &mut sections),
        None => {}
    }
    file_section(properties, locations).finish(&mut sections);
    sections
}

/// The rows of a picture's, video's or audio file's own details that
/// the Details pane lists under its file rows, as Windows 11's pane does
/// (PROP-009): the most telling ones of [`detail_sections`], in the pane's
/// order, with the tab's wording and values. Each row is its untranslated
/// name (the pane's key), its shown name and its value; an unrated file
/// has no Rating row, and a row the file does not have is left out.
pub(crate) fn pane_media_rows(kind: MediaKind, media: &MediaDetails) -> Vec<(&'static str, String, String)> {
    let mut sections = Vec::new();
    let wanted: &[&'static str] = match kind {
        MediaKind::Picture => {
            picture_sections(None, media, &mut sections);
            &[
                "Date taken",
                "Dimensions",
                "Rating",
                "Title",
                "Tags",
                "Authors",
                "Camera maker",
                "Camera model",
                "F-stop",
                "Exposure time",
                "ISO speed",
                "Exposure bias",
                "Focal length",
                "Flash mode",
            ]
        }
        MediaKind::Video => {
            video_sections(media, &mut sections);
            &[
                "Length",
                "Frame width",
                "Frame height",
                "Frame rate",
                "Data rate",
                "Total bitrate",
                "Bit rate",
                "Title",
                "Contributing artists",
                "Year",
                "Genre",
            ]
        }
        MediaKind::Audio => {
            audio_sections(media, &mut sections);
            &[
                "Length",
                "Contributing artists",
                "Album artist",
                "Album",
                "Year",
                "#",
                "Genre",
                "Bit rate",
                "Title",
            ]
        }
    };
    let rows: Vec<&DetailRow> = sections.iter().flat_map(|section| &section.rows).collect();
    wanted
        .iter()
        .filter_map(|key| {
            let name = ox_core::i18n::gettext(key);
            let row = rows.iter().find(|row| row.name == name)?;
            if row.value == DetailValue::Stars(0) {
                return None;
            }
            Some((*key, name, row.value.text()))
        })
        .collect()
}

fn rating(media: &MediaDetails) -> Option<u8> {
    media.rating.or(media.rating_readable.then_some(0))
}

fn picture_sections(dimensions: Option<(i32, i32)>, media: &MediaDetails, sections: &mut Vec<DetailSection>) {
    let mut description = Section::new(message_id("Description"));
    description
        .text(message_id("Title"), media.title.clone())
        .text(message_id("Subject"), media.subject.clone())
        .stars(message_id("Rating"), rating(media))
        .text(message_id("Tags"), media.tags.clone())
        .text(message_id("Comments"), media.comments.clone());
    description.finish(sections);

    let mut origin = Section::new(message_id("Origin"));
    origin
        .text(message_id("Authors"), media.authors.clone())
        .text(
            message_id("Date taken"),
            media
                .date_taken
                .and_then(crate::media_details::NaiveDateTime::local_unix_seconds)
                .map(|seconds| format::date_short_time_text(Some(seconds))),
        )
        .text(message_id("Program name"), media.program_name.clone())
        .text(message_id("Copyright"), media.copyright.clone());
    origin.finish(sections);

    let size = media.width.zip(media.height).or_else(|| {
        dimensions.and_then(|(width, height)| Some((u32::try_from(width).ok()?, u32::try_from(height).ok()?)))
    });
    let mut image = Section::new(message_id("Image"));
    image
        .text(
            message_id("Dimensions"),
            size.map(|(width, height)| dimensions_text(width, height)),
        )
        .text(message_id("Width"), size.map(|(width, _)| pixels_text(width)))
        .text(message_id("Height"), size.map(|(_, height)| pixels_text(height)))
        .text(
            message_id("Horizontal resolution"),
            media.resolution.map(|(horizontal, _)| dpi_text(horizontal)),
        )
        .text(
            message_id("Vertical resolution"),
            media.resolution.map(|(_, vertical)| dpi_text(vertical)),
        )
        .text(
            message_id("Bit depth"),
            media.bit_depth.map(|depth| depth.to_string()),
        )
        .text(
            message_id("Color representation"),
            media.color_space.and_then(color_space_text),
        );
    image.finish(sections);

    let mut camera = Section::new(message_id("Camera"));
    camera
        .text(message_id("Camera maker"), media.camera_maker.clone())
        .text(message_id("Camera model"), media.camera_model.clone())
        .text(message_id("F-stop"), media.f_number.map(f_stop_text))
        .text(
            message_id("Exposure time"),
            media.exposure_time.map(exposure_text),
        )
        .text(
            message_id("ISO speed"),
            media
                .iso
                .map(|iso| ox_core::i18n::format_message("ISO-{value}", &[("value", &iso.to_string())])),
        )
        .text(message_id("Exposure bias"), media.exposure_bias.map(bias_text))
        .text(
            message_id("Focal length"),
            media
                .focal_length
                .map(|length| ox_core::i18n::format_message("{value} mm", &[("value", &decimal(length, 0))])),
        )
        .text(
            message_id("Metering mode"),
            media.metering_mode.and_then(metering_text),
        )
        .text(message_id("Flash mode"), media.flash.map(flash_text))
        .text(
            message_id("35mm focal length"),
            media.focal_length_35mm.map(|length| length.to_string()),
        );
    camera.finish(sections);

    let mut advanced = Section::new(message_id("Advanced photo"));
    advanced
        .text(message_id("Lens maker"), media.lens_maker.clone())
        .text(message_id("Lens model"), media.lens_model.clone())
        .text(
            message_id("Exposure program"),
            media.exposure_program.and_then(exposure_program_text),
        )
        .text(
            message_id("White balance"),
            media.white_balance.and_then(white_balance_text),
        )
        .text(message_id("EXIF version"), media.exif_version.clone());
    advanced.finish(sections);
}

fn video_sections(media: &MediaDetails, sections: &mut Vec<DetailSection>) {
    let mut description = Section::new(message_id("Description"));
    description
        .text(message_id("Title"), media.title.clone())
        .stars(message_id("Rating"), rating(media))
        .text(message_id("Tags"), media.tags.clone())
        .text(message_id("Comments"), media.comments.clone());
    description.finish(sections);

    let mut video = Section::new(message_id("Video"));
    video
        .text(message_id("Length"), media.length.map(length_text))
        .text(
            message_id("Frame width"),
            media.frame_width.map(|width| width.to_string()),
        )
        .text(
            message_id("Frame height"),
            media.frame_height.map(|height| height.to_string()),
        )
        .text(message_id("Data rate"), media.data_rate.map(kbps_text))
        .text(message_id("Total bitrate"), media.total_bitrate.map(kbps_text))
        .text(message_id("Frame rate"), media.frame_rate.map(frame_rate_text));
    video.finish(sections);

    audio_section(media).finish(sections);

    let mut about = Section::new(message_id("Media"));
    about
        .text(message_id("Contributing artists"), media.artists.clone())
        .text(message_id("Year"), media.year.map(|year| year.to_string()))
        .text(message_id("Genre"), media.genre.clone());
    about.finish(sections);

    origin_section(media).finish(sections);
    content_section(media).finish(sections);
}

fn audio_sections(media: &MediaDetails, sections: &mut Vec<DetailSection>) {
    let mut description = Section::new(message_id("Description"));
    description
        .text(message_id("Title"), media.title.clone())
        .stars(message_id("Rating"), rating(media))
        .text(message_id("Comments"), media.comments.clone());
    description.finish(sections);

    let mut about = Section::new(message_id("Media"));
    about
        .text(message_id("Contributing artists"), media.artists.clone())
        .text(message_id("Album artist"), media.album_artist.clone())
        .text(message_id("Album"), media.album.clone())
        .text(message_id("Year"), media.year.map(|year| year.to_string()))
        .text(message_id("#"), media.track.map(|track| track.to_string()))
        .text(message_id("Genre"), media.genre.clone())
        .text(message_id("Length"), media.length.map(length_text));
    about.finish(sections);

    audio_section(media).finish(sections);
    origin_section(media).finish(sections);
    content_section(media).finish(sections);
}

fn audio_section(media: &MediaDetails) -> Section {
    let mut audio = Section::new(message_id("Audio"));
    audio
        .text(message_id("Bit rate"), media.audio_bitrate.map(kbps_text))
        .text(message_id("Channels"), media.channels.map(channels_text))
        .text(
            message_id("Audio sample rate"),
            media.sample_rate.map(sample_rate_text),
        );
    audio
}

fn origin_section(media: &MediaDetails) -> Section {
    let mut origin = Section::new(message_id("Origin"));
    origin
        .text(message_id("Publisher"), media.publisher.clone())
        .text(message_id("Encoded by"), media.encoded_by.clone())
        .text(message_id("Copyright"), media.copyright.clone());
    origin
}

fn content_section(media: &MediaDetails) -> Section {
    let mut content = Section::new(message_id("Content"));
    content
        .text(message_id("Composers"), media.composers.clone())
        .text(message_id("Conductors"), media.conductors.clone());
    content
}

/// Name, Item type, Folder path, Date created, Date modified, Size,
/// Attributes and Owner.
fn file_section(properties: &ItemProperties, locations: &LocationContext) -> Section {
    let entry = &properties.entry;
    let folder = properties.parent_uri.as_deref().map(|parent| {
        gio::File::for_uri(parent)
            .path()
            .filter(|_| parent.starts_with("file:"))
            .map_or_else(
                || locations.display_location(parent),
                |path| path.display().to_string(),
            )
    });
    let mut file = Section::new(message_id("File"));
    file.text(message_id("Name"), Some(entry.name.clone()))
        .text(message_id("Item type"), Some(entry.type_label.clone()))
        .text(message_id("Folder path"), folder)
        .text(
            message_id("Date created"),
            properties
                .created
                .map(|created| format::date_short_time_text(Some(created))),
        )
        .text(
            message_id("Date modified"),
            entry
                .modified
                .map(|modified| format::date_short_time_text(Some(modified))),
        )
        .text(
            message_id("Size"),
            entry.size.map(|size| format::size_text(Some(size))),
        )
        .text(message_id("Attributes"), attributes_text(properties))
        .text(message_id("Owner"), properties.owner.clone());
    file
}

/// Windows' attribute letters Linux has: R when the owner may not write
/// the file, H when its name starts with a dot.
fn attributes_text(properties: &ItemProperties) -> Option<String> {
    let mut letters = String::new();
    if properties.mode.is_some_and(|mode| mode & 0o200 == 0) {
        letters.push('R');
    }
    if properties.entry.name.starts_with('.') {
        letters.push('H');
    }
    (!letters.is_empty()).then_some(letters)
}

/// `1920 x 1080`.
pub(super) fn dimensions_text(width: u32, height: u32) -> String {
    ox_core::i18n::format_message(
        "{width} x {height}",
        &[("width", &width.to_string()), ("height", &height.to_string())],
    )
}

/// `1920 pixels`.
fn pixels_text(count: u32) -> String {
    ox_core::i18n::format_message(
        &ox_core::i18n::ngettext("{count} pixel", "{count} pixels", u64::from(count)),
        &[("count", &count.to_string())],
    )
}

/// `96 dpi`.
fn dpi_text(dpi: f64) -> String {
    ox_core::i18n::format_message("{value} dpi", &[("value", &decimal(dpi, 0))])
}

/// `f/2.8`.
pub(super) fn f_stop_text(f_number: f64) -> String {
    ox_core::i18n::format_message("f/{value}", &[("value", &decimal(f_number, 1))])
}

/// `1/125 sec.`, or `2 sec.` for a long exposure.
pub(super) fn exposure_text((numerator, denominator): (u32, u32)) -> String {
    let value = if numerator == 0 {
        "0".to_owned()
    } else if numerator < denominator {
        let denominator = (f64::from(denominator) / f64::from(numerator)).round();
        format!("1/{denominator}")
    } else {
        decimal(f64::from(numerator) / f64::from(denominator), 1)
    };
    ox_core::i18n::format_message("{value} sec.", &[("value", &value)])
}

/// `0 step`, `+0.7 step`, `-1 step`.
fn bias_text(bias: f64) -> String {
    let value = decimal(bias, 1);
    let value = if bias > 0.0 { format!("+{value}") } else { value };
    ox_core::i18n::format_message("{value} step", &[("value", &value)])
}

/// `00:03:25`.
pub(super) fn length_text(nanoseconds: u64) -> String {
    let seconds = (nanoseconds + 500_000_000) / 1_000_000_000;
    format!(
        "{:02}:{:02}:{:02}",
        seconds / 3600,
        seconds / 60 % 60,
        seconds % 60
    )
}

/// `320kbps`.
pub(super) fn kbps_text(bits_per_second: u32) -> String {
    let kilobits = (bits_per_second + 500) / 1000;
    ox_core::i18n::format_message("{value}kbps", &[("value", &kilobits.to_string())])
}

/// `29.97 frames/second`.
pub(super) fn frame_rate_text((numerator, denominator): (u32, u32)) -> String {
    let rate = f64::from(numerator) / f64::from(denominator.max(1));
    ox_core::i18n::format_message("{value} frames/second", &[("value", &fixed(rate, 2))])
}

/// `2 (stereo)`, `1 (mono)`, or the number of channels.
pub(super) fn channels_text(channels: u32) -> String {
    match channels {
        1 => ox_core::i18n::gettext("1 (mono)"),
        2 => ox_core::i18n::gettext("2 (stereo)"),
        count => count.to_string(),
    }
}

/// `44.100 kHz`.
pub(super) fn sample_rate_text(hertz: u32) -> String {
    ox_core::i18n::format_message("{value} kHz", &[("value", &fixed(f64::from(hertz) / 1000.0, 3))])
}

fn color_space_text(space: u16) -> Option<String> {
    match space {
        1 => Some(ox_core::i18n::gettext("sRGB")),
        0xffff => Some(ox_core::i18n::gettext("Uncalibrated")),
        _ => None,
    }
}

fn metering_text(mode: u16) -> Option<String> {
    let name = match mode {
        1 => message_id("Average"),
        2 => message_id("Center Weighted Average"),
        3 => message_id("Spot"),
        4 => message_id("Multi Spot"),
        5 => message_id("Pattern"),
        6 => message_id("Partial"),
        255 => message_id("Other"),
        _ => return None,
    };
    Some(ox_core::i18n::gettext(name))
}

/// Windows' Flash mode: whether the flash fired, and how it was set.
pub(super) fn flash_text(flash: u16) -> String {
    let fired = flash & 1 == 1;
    let name = match ((flash >> 3) & 3, fired) {
        (1, true) => message_id("Flash, compulsory"),
        (2, false) => message_id("No flash, compulsory"),
        (3, true) => message_id("Flash, auto"),
        (3, false) => message_id("No flash, auto"),
        (_, true) => message_id("Flash"),
        (_, false) => message_id("No flash"),
    };
    let text = ox_core::i18n::gettext(name);
    if flash & 0x40 == 0 {
        text
    } else {
        ox_core::i18n::format_message("{flash}, red-eye reduction", &[("flash", &text)])
    }
}

fn exposure_program_text(program: u16) -> Option<String> {
    let name = match program {
        1 => message_id("Manual"),
        2 => message_id("Normal"),
        3 => message_id("Aperture priority"),
        4 => message_id("Shutter priority"),
        5 => message_id("Creative program"),
        6 => message_id("Action program"),
        7 => message_id("Portrait mode"),
        8 => message_id("Landscape mode"),
        _ => return None,
    };
    Some(ox_core::i18n::gettext(name))
}

fn white_balance_text(balance: u16) -> Option<String> {
    match balance {
        0 => Some(ox_core::i18n::gettext("Auto")),
        1 => Some(ox_core::i18n::gettext("Manual")),
        _ => None,
    }
}

/// `value` with at most `places` decimals, without trailing zeros past
/// the point (`2.8`, `96`), in the locale's decimal mark.
fn decimal(value: f64, places: usize) -> String {
    let text = format!("{value:.places$}");
    let text = if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.').to_owned()
    } else {
        text
    };
    with_decimal_mark(&text)
}

/// `value` with exactly `places` decimals (`30.00`), in the locale's
/// decimal mark, as Windows writes a frame rate and a sample rate.
fn fixed(value: f64, places: usize) -> String {
    with_decimal_mark(&format!("{value:.places$}"))
}

fn with_decimal_mark(text: &str) -> String {
    let mark = ox_core::i18n::pgettext("decimal separator", ".");
    text.replace('.', &mark)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media_details::NaiveDateTime;
    use crate::test_support::file_entry;

    fn properties(name: &str, mode: u32) -> ItemProperties {
        let mut entry = file_entry(name);
        entry.size = Some(322_003);
        entry.type_label = "JPEG image".to_owned();
        ItemProperties {
            entry,
            parent_uri: Some("file:///tmp/ox-test".to_owned()),
            created: None,
            accessed: None,
            owner: Some("tester".to_owned()),
            group: None,
            uid: None,
            gid: None,
            mode: Some(mode),
            access: super::super::metadata::Access::default(),
            link_target: None,
            links_to_folder: false,
            default_app: None,
            mount: None,
            dimensions: None,
            custom_icon: None,
            size_on_disk: None,
        }
    }

    fn headings(sections: &[DetailSection]) -> Vec<&str> {
        sections.iter().map(|section| section.heading.as_str()).collect()
    }

    fn row<'a>(sections: &'a [DetailSection], name: &str) -> Option<&'a DetailValue> {
        sections
            .iter()
            .flat_map(|section| &section.rows)
            .find(|row| row.name == name)
            .map(|row| &row.value)
    }

    fn text(sections: &[DetailSection], name: &str) -> Option<String> {
        row(sections, name).map(DetailValue::text)
    }

    /// A photo's rows come under Windows' headings in its order, with its
    /// wording; headings without rows are left out.
    ///
    /// parity: PROP-034
    #[test]
    fn a_photos_rows_follow_windows() {
        let media = MediaDetails {
            title: Some("Harbour at dusk".to_owned()),
            rating: Some(4),
            rating_readable: true,
            authors: Some("A. Example".to_owned()),
            date_taken: NaiveDateTime::parse_exif("2026:05:01 18:42:07"),
            width: Some(4000),
            height: Some(3000),
            resolution: Some((72.0, 72.0)),
            bit_depth: Some(24),
            color_space: Some(1),
            camera_maker: Some("Fictional Optics".to_owned()),
            camera_model: Some("FO-100".to_owned()),
            f_number: Some(2.8),
            exposure_time: Some((1, 125)),
            iso: Some(200),
            exposure_bias: Some(0.0),
            focal_length: Some(50.0),
            metering_mode: Some(5),
            flash: Some(0x10),
            focal_length_35mm: Some(75),
            ..MediaDetails::default()
        };
        let sections = detail_sections(
            &properties("Harbour.jpg", 0o100_444),
            Some(MediaKind::Picture),
            Some(&media),
            &LocationContext::default(),
        );

        assert_eq!(
            headings(&sections),
            ["Description", "Origin", "Image", "Camera", "File"]
        );
        let names: Vec<&str> = sections[2].rows.iter().map(|row| row.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "Dimensions",
                "Width",
                "Height",
                "Horizontal resolution",
                "Vertical resolution",
                "Bit depth",
                "Color representation"
            ]
        );
        assert_eq!(row(&sections, "Rating"), Some(&DetailValue::Stars(4)));
        assert_eq!(text(&sections, "Rating").as_deref(), Some("4 stars"));
        assert_eq!(text(&sections, "Dimensions").as_deref(), Some("4000 x 3000"));
        assert_eq!(text(&sections, "Width").as_deref(), Some("4000 pixels"));
        assert_eq!(
            text(&sections, "Horizontal resolution").as_deref(),
            Some("72 dpi")
        );
        assert_eq!(text(&sections, "Color representation").as_deref(), Some("sRGB"));
        assert_eq!(text(&sections, "F-stop").as_deref(), Some("f/2.8"));
        assert_eq!(text(&sections, "Exposure time").as_deref(), Some("1/125 sec."));
        assert_eq!(text(&sections, "ISO speed").as_deref(), Some("ISO-200"));
        assert_eq!(text(&sections, "Exposure bias").as_deref(), Some("0 step"));
        assert_eq!(text(&sections, "Focal length").as_deref(), Some("50 mm"));
        assert_eq!(text(&sections, "Metering mode").as_deref(), Some("Pattern"));
        assert_eq!(
            text(&sections, "Flash mode").as_deref(),
            Some("No flash, compulsory")
        );
        assert_eq!(text(&sections, "35mm focal length").as_deref(), Some("75"));
        assert!(text(&sections, "Date taken").is_some_and(|date| date.contains("2026")));
        assert_eq!(text(&sections, "Folder path").as_deref(), Some("/tmp/ox-test"));
        assert_eq!(
            text(&sections, "Size").as_deref(),
            Some(format::size_text(Some(322_003)).as_str())
        );
        assert_eq!(text(&sections, "Attributes").as_deref(), Some("R"));
        assert_eq!(text(&sections, "Owner").as_deref(), Some("tester"));
        assert_eq!(
            row(&sections, "Lens model"),
            None,
            "no Advanced photo without its tags"
        );
    }

    /// An unrated picture shows five empty stars; a picture whose
    /// reader failed still shows the size gdk-pixbuf read.
    ///
    /// parity: PROP-034
    #[test]
    fn an_unrated_picture_shows_empty_stars() {
        let unrated = MediaDetails {
            rating_readable: true,
            ..MediaDetails::default()
        };
        let mut item = properties("Screenshot.png", 0o100_644);
        let sections = detail_sections(
            &item,
            Some(MediaKind::Picture),
            Some(&unrated),
            &LocationContext::default(),
        );
        assert_eq!(row(&sections, "Rating"), Some(&DetailValue::Stars(0)));
        assert_eq!(text(&sections, "Rating").as_deref(), Some("Unrated"));
        assert_eq!(
            text(&sections, "Attributes"),
            None,
            "a writable, visible file has no letters"
        );

        item.dimensions = Some((485, 596));
        let sections = detail_sections(&item, Some(MediaKind::Picture), None, &LocationContext::default());
        assert_eq!(headings(&sections), ["Image", "File"]);
        assert_eq!(text(&sections, "Dimensions").as_deref(), Some("485 x 596"));
    }

    /// A video's and an audio file's headings and values, as Windows
    /// writes them.
    ///
    /// parity: PROP-034
    #[test]
    fn video_and_audio_rows_follow_windows() {
        let media = MediaDetails {
            title: Some("Harbour walk".to_owned()),
            rating_readable: true,
            length: Some(205_400_000_000),
            frame_width: Some(1920),
            frame_height: Some(1080),
            data_rate: Some(1_234_000),
            total_bitrate: Some(1_362_000),
            frame_rate: Some((30_000, 1001)),
            audio_bitrate: Some(128_000),
            channels: Some(2),
            sample_rate: Some(48_000),
            artists: Some("The Example Quartet".to_owned()),
            album_artist: Some("Example Ensemble".to_owned()),
            album: Some("Coastal Sketches".to_owned()),
            year: Some(2025),
            track: Some(3),
            genre: Some("Folk".to_owned()),
            encoded_by: Some("Example Encoder".to_owned()),
            composers: Some("B. Placeholder".to_owned()),
            ..MediaDetails::default()
        };
        let item = properties("Harbour walk.mp4", 0o100_644);
        let video = detail_sections(
            &item,
            Some(MediaKind::Video),
            Some(&media),
            &LocationContext::default(),
        );
        assert_eq!(
            headings(&video),
            [
                "Description",
                "Video",
                "Audio",
                "Media",
                "Origin",
                "Content",
                "File"
            ]
        );
        assert_eq!(text(&video, "Length").as_deref(), Some("00:03:25"));
        assert_eq!(text(&video, "Frame width").as_deref(), Some("1920"));
        assert_eq!(text(&video, "Data rate").as_deref(), Some("1234kbps"));
        assert_eq!(text(&video, "Total bitrate").as_deref(), Some("1362kbps"));
        assert_eq!(text(&video, "Frame rate").as_deref(), Some("29.97 frames/second"));
        assert_eq!(text(&video, "Channels").as_deref(), Some("2 (stereo)"));
        assert_eq!(text(&video, "Audio sample rate").as_deref(), Some("48.000 kHz"));
        assert_eq!(row(&video, "Album"), None, "a video's Media has no album");

        let audio = detail_sections(
            &item,
            Some(MediaKind::Audio),
            Some(&media),
            &LocationContext::default(),
        );
        assert_eq!(
            headings(&audio),
            ["Description", "Media", "Audio", "Origin", "Content", "File"]
        );
        let names: Vec<&str> = audio[1].rows.iter().map(|row| row.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "Contributing artists",
                "Album artist",
                "Album",
                "Year",
                "#",
                "Genre",
                "Length"
            ]
        );
        assert_eq!(text(&audio, "Bit rate").as_deref(), Some("128kbps"));
        assert_eq!(text(&audio, "Encoded by").as_deref(), Some("Example Encoder"));
    }

    /// Any other file has only File, and a hidden one says H.
    ///
    /// parity: PROP-034
    #[test]
    fn other_files_have_only_their_file_rows() {
        let sections = detail_sections(
            &properties(".notes.txt", 0o100_600),
            None,
            None,
            &LocationContext::default(),
        );
        assert_eq!(headings(&sections), ["File"]);
        let names: Vec<&str> = sections[0].rows.iter().map(|row| row.name.as_str()).collect();
        assert_eq!(
            names,
            ["Name", "Item type", "Folder path", "Size", "Attributes", "Owner"]
        );
        assert_eq!(text(&sections, "Attributes").as_deref(), Some("H"));
    }

    /// Exposure, flash and channel wording.
    #[test]
    fn camera_values_read_as_windows_writes_them() {
        assert_eq!(exposure_text((10, 2500)), "1/250 sec.");
        assert_eq!(exposure_text((2, 1)), "2 sec.");
        assert_eq!(exposure_text((5, 2)), "2.5 sec.");
        assert_eq!(flash_text(0x19), "Flash, auto");
        assert_eq!(flash_text(0x18), "No flash, auto");
        assert_eq!(flash_text(0x01), "Flash");
        assert_eq!(flash_text(0x49), "Flash, compulsory, red-eye reduction");
        assert_eq!(channels_text(1), "1 (mono)");
        assert_eq!(channels_text(6), "6");
        assert_eq!(sample_rate_text(44_100), "44.100 kHz");
        assert_eq!(f_stop_text(4.0), "f/4");
        assert_eq!(length_text(59_600_000_000), "00:01:00");
        assert_eq!(kbps_text(320_000), "320kbps");
    }

    /// The Details pane takes a photo's telling rows from the tab, in its
    /// own order and with the tab's values; an unrated file has no Rating.
    ///
    /// parity: PROP-009, PROP-034
    #[test]
    fn the_pane_lists_a_photos_telling_rows_in_its_order() {
        let media = MediaDetails {
            rating_readable: true,
            date_taken: NaiveDateTime::parse_exif("2026:05:01 18:42:07"),
            width: Some(4000),
            height: Some(3000),
            resolution: Some((72.0, 72.0)),
            camera_maker: Some("Fictional Optics".to_owned()),
            camera_model: Some("FO-100".to_owned()),
            f_number: Some(2.8),
            exposure_time: Some((1, 125)),
            iso: Some(200),
            ..MediaDetails::default()
        };
        let rows = pane_media_rows(MediaKind::Picture, &media);
        let keys: Vec<&str> = rows.iter().map(|(key, _, _)| *key).collect();
        assert_eq!(
            keys,
            [
                "Date taken",
                "Dimensions",
                "Camera maker",
                "Camera model",
                "F-stop",
                "Exposure time",
                "ISO speed"
            ]
        );
        let value = |key: &str| rows.iter().find(|row| row.0 == key).map(|row| row.2.clone());
        assert_eq!(value("Dimensions").as_deref(), Some("4000 x 3000"));
        assert_eq!(value("F-stop").as_deref(), Some("f/2.8"));
        assert_eq!(value("Exposure time").as_deref(), Some("1/125 sec."));
        assert_eq!(value("ISO speed").as_deref(), Some("ISO-200"));

        let song = MediaDetails {
            length: Some(205_000_000_000),
            artists: Some("The Example Band".to_owned()),
            audio_bitrate: Some(320_000),
            ..MediaDetails::default()
        };
        let keys: Vec<&str> = pane_media_rows(MediaKind::Audio, &song)
            .iter()
            .map(|(key, _, _)| *key)
            .collect();
        assert_eq!(keys, ["Length", "Contributing artists", "Bit rate"]);
    }
}
