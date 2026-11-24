// SPDX-License-Identifier: AGPL-3.0-only
//! Office documents written by the tests themselves, so no third-party
//! file is ever read: the smallest Word, Excel and `PowerPoint` files and
//! `OpenDocument` text, spreadsheets and presentations `LibreOffice` opens,
//! each a ZIP archive of a few XML parts written here without
//! compression; one with a Basic macro that would replace its text when
//! it is opened; one that asks for a password; a damaged one; and noise
//! under a document's name.

use std::fmt::Write as _;
use std::fs;
use std::path::Path;

/// The text the test macro would put in its document.
pub(crate) const MACRO_MARK: &str = "MACRO RAN";

/// The kinds of document written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Docx,
    Xlsx,
    Pptx,
    Odt,
    Ods,
    Odp,
}

impl Kind {
    /// Every kind.
    pub(crate) const ALL: [Kind; 6] = [
        Kind::Docx,
        Kind::Xlsx,
        Kind::Pptx,
        Kind::Odt,
        Kind::Ods,
        Kind::Odp,
    ];

    /// The extension a file of this kind has.
    pub(crate) fn extension(self) -> &'static str {
        match self {
            Kind::Docx => "docx",
            Kind::Xlsx => "xlsx",
            Kind::Pptx => "pptx",
            Kind::Odt => "odt",
            Kind::Ods => "ods",
            Kind::Odp => "odp",
        }
    }

    /// The type GIO names a file of this kind.
    pub(crate) fn content_type(self) -> &'static str {
        match self {
            Kind::Docx => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            Kind::Xlsx => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            Kind::Pptx => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
            Kind::Odt => "application/vnd.oasis.opendocument.text",
            Kind::Ods => "application/vnd.oasis.opendocument.spreadsheet",
            Kind::Odp => "application/vnd.oasis.opendocument.presentation",
        }
    }
}

/// Writes a document of `kind` at `path` that shows `text`.
pub(crate) fn write(path: &Path, kind: Kind, text: &str) {
    write_paragraphs(path, kind, &[text.to_owned()]);
}

/// Writes a document of `kind` at `path` with `count` numbered
/// paragraphs (rows, slides), long enough to fill many pages.
pub(crate) fn write_long(path: &Path, kind: Kind, count: usize) {
    let filler = "The committee reviewed the fictional figures line by line and agreed on next steps. ";
    let paragraphs: Vec<String> = (1..=count)
        .map(|number| format!("Paragraph {number}. {}", filler.repeat(4)))
        .collect();
    write_paragraphs(path, kind, &paragraphs);
}

/// Writes a document of `kind` at `path` with one paragraph, row or
/// slide for each of `paragraphs`.
pub(crate) fn write_paragraphs(path: &Path, kind: Kind, paragraphs: &[String]) {
    let parts = match kind {
        Kind::Docx => docx(paragraphs),
        Kind::Xlsx => xlsx(paragraphs),
        Kind::Pptx => pptx(paragraphs),
        Kind::Odt => {
            let mut body = String::new();
            for text in paragraphs {
                let _ = write!(body, "<text:p>{}</text:p>", escape(text));
            }
            odf(ODT, &format!("<office:text>{body}</office:text>"), "", &[])
        }
        Kind::Ods => {
            let mut rows = String::new();
            for text in paragraphs {
                let _ = write!(
                    rows,
                    "<table:table-row><table:table-cell office:value-type=\"string\"><text:p>{}</text:p></table:table-cell></table:table-row>",
                    escape(text)
                );
            }
            odf(
                ODS,
                &format!("<office:spreadsheet><table:table table:name=\"Sheet1\">{rows}</table:table></office:spreadsheet>"),
                "",
                &[],
            )
        }
        Kind::Odp => {
            let mut pages = String::new();
            for (index, text) in paragraphs.iter().enumerate() {
                let _ = write!(
                    pages,
                        "<draw:page draw:name=\"page{}\"><draw:frame svg:x=\"2cm\" svg:y=\"2cm\" svg:width=\"22cm\" svg:height=\"12cm\"><draw:text-box><text:p>{}</text:p></draw:text-box></draw:frame></draw:page>",
                    index + 1,
                    escape(text)
                );
            }
            odf(
                ODP,
                &format!("<office:presentation>{pages}</office:presentation>"),
                "",
                &[],
            )
        }
    };
    fs::write(path, zip(&parts)).expect("the test document");
}

/// Writes an `OpenDocument` text at `path` that shows `text`, with a Basic
/// macro bound to its opening that would replace the text with
/// [`MACRO_MARK`].
pub(crate) fn write_with_macro(path: &Path, text: &str) {
    let module = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?><!DOCTYPE script:module PUBLIC \"-//OpenOffice.org//DTD OfficeDocument 1.0//EN\" \"module.dtd\"><script:module xmlns:script=\"http://openoffice.org/2000/script\" script:name=\"Module1\" script:language=\"StarBasic\">Sub Main\n ThisComponent.getText().setString(\"{MACRO_MARK}\")\nEnd Sub\n</script:module>"
    );
    let library = "<?xml version=\"1.0\" encoding=\"UTF-8\"?><!DOCTYPE library:library PUBLIC \"-//OpenOffice.org//DTD OfficeDocument 1.0//EN\" \"library.dtd\"><library:library xmlns:library=\"http://openoffice.org/2000/library\" library:name=\"Standard\" library:readonly=\"false\" library:passwordprotected=\"false\"><library:element library:name=\"Module1\"/></library:library>";
    let libraries = "<?xml version=\"1.0\" encoding=\"UTF-8\"?><!DOCTYPE library:libraries PUBLIC \"-//OpenOffice.org//DTD OfficeDocument 1.0//EN\" \"libraries.dtd\"><library:libraries xmlns:library=\"http://openoffice.org/2000/library\" xmlns:xlink=\"http://www.w3.org/1999/xlink\"><library:library library:name=\"Standard\" library:link=\"false\"/></library:libraries>";
    let events = "<office:scripts><office:event-listeners><script:event-listener script:language=\"ooo:script\" script:event-name=\"dom:load\" xlink:href=\"vnd.sun.star.script:Standard.Module1.Main?language=Basic&amp;location=document\" xlink:type=\"simple\"/></office:event-listeners></office:scripts>";
    let parts = odf(
        ODT,
        &format!("<office:text><text:p>{}</text:p></office:text>", escape(text)),
        events,
        &[
            ("Basic/Standard/Module1.xml", module.into_bytes()),
            ("Basic/Standard/script-lb.xml", library.as_bytes().to_vec()),
            ("Basic/script-lc.xml", libraries.as_bytes().to_vec()),
        ],
    );
    fs::write(path, zip(&parts)).expect("the test document");
}

/// Writes an `OpenDocument` text at `path` whose content is encrypted, as
/// one saved with a password is: its manifest says so, and its content
/// is bytes no password opens, so `LibreOffice` must ask for one.
pub(crate) fn write_password_protected(path: &Path) {
    let manifest = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?><manifest:manifest xmlns:manifest=\"urn:oasis:names:tc:opendocument:xmlns:manifest:1.0\" manifest:version=\"1.2\"><manifest:file-entry manifest:full-path=\"/\" manifest:media-type=\"{ODT}\"/><manifest:file-entry manifest:full-path=\"content.xml\" manifest:media-type=\"text/xml\" manifest:size=\"4096\"><manifest:encryption-data manifest:checksum-type=\"urn:oasis:names:tc:opendocument:xmlns:manifest:1.0#sha256-1k\" manifest:checksum=\"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=\"><manifest:algorithm manifest:algorithm-name=\"http://www.w3.org/2001/04/xmlenc#aes256-cbc\" manifest:initialisation-vector=\"AAAAAAAAAAAAAAAAAAAAAA==\"/><manifest:start-key-generation manifest:start-key-generation-name=\"http://www.w3.org/2000/09/xmldsig#sha256\" manifest:key-size=\"32\"/><manifest:key-derivation manifest:key-derivation-name=\"PBKDF2\" manifest:key-size=\"32\" manifest:iteration-count=\"100000\" manifest:salt=\"AAAAAAAAAAAAAAAAAAAAAA==\"/></manifest:encryption-data></manifest:file-entry></manifest:manifest>"
    );
    let parts = vec![
        ("mimetype".to_owned(), ODT.as_bytes().to_vec()),
        ("META-INF/manifest.xml".to_owned(), manifest.into_bytes()),
        ("content.xml".to_owned(), noise(4096, 7)),
    ];
    fs::write(path, zip(&parts)).expect("the test document");
}

/// Writes a Word document at `path` cut off two thirds of the way, as an
/// interrupted download leaves one.
pub(crate) fn write_damaged(path: &Path) {
    let whole = zip(&docx(&["A document that was cut off".to_owned()]));
    fs::write(path, &whole[..whole.len() * 2 / 3]).expect("the test document");
}

/// Writes an Excel workbook at `path` of `count` formulas that each sum
/// over a whole column: a file of a few kilobytes that keeps `LibreOffice`
/// calculating for many seconds (600 take about half a minute).
pub(crate) fn write_slow_spreadsheet(path: &Path, count: usize) {
    let mut parts = xlsx(&[]);
    let mut data = String::new();
    for row in 1..=count {
        let _ = write!(
            data,
            "<row r=\"{row}\"><c r=\"A{row}\"><f>SUMPRODUCT((ROW(B1:B1048576)&gt;{row})*1)</f></c></row>"
        );
    }
    let sheet = parts
        .iter_mut()
        .find(|(name, _)| name == "xl/worksheets/sheet1.xml")
        .expect("the sheet");
    sheet.1 = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\"><sheetData>{data}</sheetData></worksheet>"
    )
    .into_bytes();
    fs::write(path, zip(&parts)).expect("the test document");
}

/// Writes a Word document at `path` of about two megabytes whose text
/// unpacks to `megabytes` mebibytes, as a decompression bomb does.
pub(crate) fn write_bomb(path: &Path, megabytes: usize) {
    use gtk::gio;
    use gtk::prelude::*;

    let paragraph = format!("<w:p><w:r><w:t>{}</w:t></w:r></w:p>", "A".repeat(2000));
    let chunk = paragraph.repeat(1024 * 1024 / paragraph.len() + 1);
    let head =
        b"<w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:body>";
    let tail = b"</w:body></w:document>";
    let compressor = gio::ZlibCompressor::new(gio::ZlibCompressorFormat::Raw, 9);
    let memory = gio::MemoryOutputStream::new_resizable();
    let stream = gio::ConverterOutputStream::new(&memory, &compressor);
    let mut crc = Crc32::new();
    let mut size = 0_usize;
    let mut write = |bytes: &[u8]| {
        stream
            .write_all(bytes, gio::Cancellable::NONE)
            .expect("compressed");
        crc.add(bytes);
        size += bytes.len();
    };
    write(head);
    for _ in 0..megabytes {
        write(chunk.as_bytes());
    }
    write(tail);
    stream.close(gio::Cancellable::NONE).expect("closed");
    let deflated = memory.steal_as_bytes().to_vec();
    let mut parts: Vec<Entry> = docx(&[])
        .into_iter()
        .filter(|(name, _)| name != "word/document.xml")
        .map(|(name, bytes)| Entry::stored(name, bytes))
        .collect();
    parts.push(Entry {
        name: "word/document.xml".to_owned(),
        crc: crc.finish(),
        size: u32::try_from(size).expect("under 4 GiB"),
        method: 8,
        bytes: deflated,
    });
    fs::write(path, zip_entries(&parts)).expect("the test document");
}

/// Writes 64 KiB of noise at `path`.
pub(crate) fn write_noise(path: &Path) {
    fs::write(path, noise(64 * 1024, 11)).expect("the test file");
}

/// The text of the PDF at `pdf` by poppler's `pdftotext`, when this
/// computer has it; the test's own document, so it runs unsandboxed.
pub(crate) fn pdf_text(pdf: &Path) -> Option<String> {
    let output = std::process::Command::new("pdftotext")
        .arg(pdf)
        .arg("-")
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

/// `length` bytes of noise from `seed`, the same each time.
fn noise(length: usize, seed: u32) -> Vec<u8> {
    let mut state = seed.wrapping_mul(2_654_435_761).max(1);
    (0..length)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state.to_le_bytes()[0]
        })
        .collect()
}

/// `text` with XML's special characters escaped.
fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

const ODT: &str = "application/vnd.oasis.opendocument.text";
const ODS: &str = "application/vnd.oasis.opendocument.spreadsheet";
const ODP: &str = "application/vnd.oasis.opendocument.presentation";

/// The namespaces of an `OpenDocument`'s content.
const ODF_NAMESPACES: &str = "xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\" xmlns:text=\"urn:oasis:names:tc:opendocument:xmlns:text:1.0\" xmlns:table=\"urn:oasis:names:tc:opendocument:xmlns:table:1.0\" xmlns:draw=\"urn:oasis:names:tc:opendocument:xmlns:drawing:1.0\" xmlns:svg=\"urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0\" xmlns:script=\"urn:oasis:names:tc:opendocument:xmlns:script:1.0\" xmlns:xlink=\"http://www.w3.org/1999/xlink\" xmlns:ooo=\"http://openoffice.org/2004/office\"";

type Parts = Vec<(String, Vec<u8>)>;

/// The parts of an `OpenDocument` of type `mime` with `body` in its
/// content, `scripts` before it, and `extra` parts.
fn odf(mime: &str, body: &str, scripts: &str, extra: &[(&str, Vec<u8>)]) -> Parts {
    let mut manifest = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?><manifest:manifest xmlns:manifest=\"urn:oasis:names:tc:opendocument:xmlns:manifest:1.0\" manifest:version=\"1.2\"><manifest:file-entry manifest:full-path=\"/\" manifest:media-type=\"{mime}\"/><manifest:file-entry manifest:full-path=\"content.xml\" manifest:media-type=\"text/xml\"/>"
    );
    for (name, _) in extra {
        let _ = write!(
            manifest,
            "<manifest:file-entry manifest:full-path=\"{name}\" manifest:media-type=\"text/xml\"/>"
        );
    }
    manifest.push_str("</manifest:manifest>");
    let content = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?><office:document-content {ODF_NAMESPACES} office:version=\"1.2\">{scripts}<office:body>{body}</office:body></office:document-content>"
    );
    let mut parts = vec![
        ("mimetype".to_owned(), mime.as_bytes().to_vec()),
        ("META-INF/manifest.xml".to_owned(), manifest.into_bytes()),
        ("content.xml".to_owned(), content.into_bytes()),
    ];
    parts.extend(
        extra
            .iter()
            .map(|(name, bytes)| ((*name).to_owned(), bytes.clone())),
    );
    parts
}

const RELATIONSHIPS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

/// An Office package's `[Content_Types].xml` with `overrides` of part
/// and type.
fn content_types(overrides: &[(&str, &str)]) -> Vec<u8> {
    let mut types = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/>",
    );
    for (part, content_type) in overrides {
        let _ = write!(
            types,
            "<Override PartName=\"{part}\" ContentType=\"{content_type}\"/>"
        );
    }
    types.push_str("</Types>");
    types.into_bytes()
}

/// An Office package's relationships, as `(id, kind, target)`.
fn relationships(items: &[(&str, &str, &str)]) -> Vec<u8> {
    let mut rels = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">",
    );
    for (id, kind, target) in items {
        let _ = write!(
            rels,
            "<Relationship Id=\"{id}\" Type=\"{RELATIONSHIPS}/{kind}\" Target=\"{target}\"/>"
        );
    }
    rels.push_str("</Relationships>");
    rels.into_bytes()
}

fn docx(paragraphs: &[String]) -> Parts {
    let mut body = String::new();
    for text in paragraphs {
        let _ = write!(body, "<w:p><w:r><w:t>{}</w:t></w:r></w:p>", escape(text));
    }
    vec![
        (
            "[Content_Types].xml".to_owned(),
            content_types(&[(
                "/word/document.xml",
                "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml",
            )]),
        ),
        (
            "_rels/.rels".to_owned(),
            relationships(&[("rId1", "officeDocument", "word/document.xml")]),
        ),
        (
            "word/document.xml".to_owned(),
            format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:body>{body}</w:body></w:document>"
            )
            .into_bytes(),
        ),
    ]
}

fn xlsx(rows: &[String]) -> Parts {
    let main = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
    let mut data = String::new();
    for (index, text) in rows.iter().enumerate() {
        let row = index + 1;
        let _ = write!(
            data,
            "<row r=\"{row}\"><c r=\"A{row}\" t=\"inlineStr\"><is><t>{}</t></is></c></row>",
            escape(text)
        );
    }
    vec![
        (
            "[Content_Types].xml".to_owned(),
            content_types(&[
                (
                    "/xl/workbook.xml",
                    "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml",
                ),
                (
                    "/xl/worksheets/sheet1.xml",
                    "application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml",
                ),
            ]),
        ),
        (
            "_rels/.rels".to_owned(),
            relationships(&[("rId1", "officeDocument", "xl/workbook.xml")]),
        ),
        (
            "xl/workbook.xml".to_owned(),
            format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><workbook xmlns=\"{main}\" xmlns:r=\"{RELATIONSHIPS}\"><sheets><sheet name=\"Sheet1\" sheetId=\"1\" r:id=\"rId1\"/></sheets></workbook>"
            )
            .into_bytes(),
        ),
        (
            "xl/_rels/workbook.xml.rels".to_owned(),
            relationships(&[("rId1", "worksheet", "worksheets/sheet1.xml")]),
        ),
        (
            "xl/worksheets/sheet1.xml".to_owned(),
            format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><worksheet xmlns=\"{main}\"><sheetData>{data}</sheetData></worksheet>"
            )
            .into_bytes(),
        ),
    ]
}

fn pptx(slides: &[String]) -> Parts {
    let presentation = "http://schemas.openxmlformats.org/presentationml/2006/main";
    let drawing = "http://schemas.openxmlformats.org/drawingml/2006/main";
    let mut overrides = vec![(
        "/ppt/presentation.xml".to_owned(),
        "application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml",
    )];
    let mut slide_ids = String::new();
    let mut slide_rels = Vec::new();
    let mut parts: Parts = Vec::new();
    for (index, text) in slides.iter().enumerate() {
        let number = index + 1;
        overrides.push((
            format!("/ppt/slides/slide{number}.xml"),
            "application/vnd.openxmlformats-officedocument.presentationml.slide+xml",
        ));
        let _ = write!(
            slide_ids,
            "<p:sldId id=\"{}\" r:id=\"rId{number}\"/>",
            255 + number
        );
        slide_rels.push((format!("rId{number}"), format!("slides/slide{number}.xml")));
        parts.push((
            format!("ppt/slides/slide{number}.xml"),
            format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><p:sld xmlns:p=\"{presentation}\" xmlns:r=\"{RELATIONSHIPS}\" xmlns:a=\"{drawing}\"><p:cSld><p:spTree><p:nvGrpSpPr><p:cNvPr id=\"1\" name=\"\"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/><p:sp><p:nvSpPr><p:cNvPr id=\"2\" name=\"Title\"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm><a:off x=\"457200\" y=\"457200\"/><a:ext cx=\"8229600\" cy=\"4572000\"/></a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></p:spPr><p:txBody><a:bodyPr wrap=\"square\"/><a:p><a:r><a:rPr lang=\"en-US\" sz=\"3600\"/><a:t>{}</a:t></a:r></a:p></p:txBody></p:sp></p:spTree></p:cSld></p:sld>",
                escape(text)
            )
            .into_bytes(),
        ));
    }
    let overrides: Vec<(&str, &str)> = overrides
        .iter()
        .map(|(part, kind)| (part.as_str(), *kind))
        .collect();
    let rels: Vec<(&str, &str, &str)> = slide_rels
        .iter()
        .map(|(id, target)| (id.as_str(), "slide", target.as_str()))
        .collect();
    let mut package = vec![
        ("[Content_Types].xml".to_owned(), content_types(&overrides)),
        (
            "_rels/.rels".to_owned(),
            relationships(&[("rId1", "officeDocument", "ppt/presentation.xml")]),
        ),
        (
            "ppt/presentation.xml".to_owned(),
            format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><p:presentation xmlns:p=\"{presentation}\" xmlns:r=\"{RELATIONSHIPS}\" xmlns:a=\"{drawing}\"><p:sldIdLst>{slide_ids}</p:sldIdLst><p:sldSz cx=\"9144000\" cy=\"6858000\"/><p:notesSz cx=\"6858000\" cy=\"9144000\"/></p:presentation>"
            )
            .into_bytes(),
        ),
        ("ppt/_rels/presentation.xml.rels".to_owned(), relationships(&rels)),
    ];
    package.extend(parts);
    package
}

/// A part of a ZIP archive: its name, the checksum and size of its
/// bytes unpacked, how they are packed (0 stored, 8 deflated) and the
/// bytes as packed.
struct Entry {
    name: String,
    crc: u32,
    size: u32,
    method: u16,
    bytes: Vec<u8>,
}

impl Entry {
    /// `bytes` stored as `name`, unpacked.
    fn stored(name: String, bytes: Vec<u8>) -> Self {
        let mut crc = Crc32::new();
        crc.add(&bytes);
        Self {
            name,
            crc: crc.finish(),
            size: u32::try_from(bytes.len()).expect("a small part"),
            method: 0,
            bytes,
        }
    }
}

/// A ZIP archive of `parts`, in their order, without compression.
fn zip(parts: &[(String, Vec<u8>)]) -> Vec<u8> {
    let entries: Vec<Entry> = parts
        .iter()
        .map(|(name, bytes)| Entry::stored(name.clone(), bytes.clone()))
        .collect();
    zip_entries(&entries)
}

/// A ZIP archive of `entries`, in their order.
fn zip_entries(entries: &[Entry]) -> Vec<u8> {
    let mut archive = Vec::new();
    let mut directory = Vec::new();
    let to_u32 = |value: usize| u32::try_from(value).expect("a small archive");
    let to_u16 = |value: usize| u16::try_from(value).expect("a short name");
    for entry in entries {
        let offset = to_u32(archive.len());
        let packed = to_u32(entry.bytes.len());
        let name_length = to_u16(entry.name.len());
        let method = entry.method.to_le_bytes();
        // The local header: version 2.0, no flags, the method, a fixed time.
        archive.extend_from_slice(&0x0403_4b50_u32.to_le_bytes());
        archive.extend_from_slice(&[20, 0, 0, 0, method[0], method[1], 0, 0, 0x21, 0x00]);
        archive.extend_from_slice(&entry.crc.to_le_bytes());
        archive.extend_from_slice(&packed.to_le_bytes());
        archive.extend_from_slice(&entry.size.to_le_bytes());
        archive.extend_from_slice(&name_length.to_le_bytes());
        archive.extend_from_slice(&0_u16.to_le_bytes());
        archive.extend_from_slice(entry.name.as_bytes());
        archive.extend_from_slice(&entry.bytes);
        directory.extend_from_slice(&0x0201_4b50_u32.to_le_bytes());
        directory.extend_from_slice(&[20, 0, 20, 0, 0, 0, method[0], method[1], 0, 0, 0x21, 0x00]);
        directory.extend_from_slice(&entry.crc.to_le_bytes());
        directory.extend_from_slice(&packed.to_le_bytes());
        directory.extend_from_slice(&entry.size.to_le_bytes());
        directory.extend_from_slice(&name_length.to_le_bytes());
        // No extra field or comment, disk 0, no attributes.
        directory.extend_from_slice(&[0; 12]);
        directory.extend_from_slice(&offset.to_le_bytes());
        directory.extend_from_slice(entry.name.as_bytes());
    }
    let directory_offset = to_u32(archive.len());
    let count = to_u16(entries.len());
    archive.extend_from_slice(&directory);
    archive.extend_from_slice(&0x0605_4b50_u32.to_le_bytes());
    archive.extend_from_slice(&[0, 0, 0, 0]);
    archive.extend_from_slice(&count.to_le_bytes());
    archive.extend_from_slice(&count.to_le_bytes());
    archive.extend_from_slice(&to_u32(directory.len()).to_le_bytes());
    archive.extend_from_slice(&directory_offset.to_le_bytes());
    archive.extend_from_slice(&0_u16.to_le_bytes());
    archive
}

/// The CRC-32 a ZIP archive keeps of a part's bytes, added in pieces.
struct Crc32(u32);

impl Crc32 {
    fn new() -> Self {
        Self(!0)
    }

    fn add(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.0 ^= u32::from(byte);
            for _ in 0..8 {
                self.0 = if self.0 & 1 == 1 {
                    (self.0 >> 1) ^ 0xedb8_8320
                } else {
                    self.0 >> 1
                };
            }
        }
    }

    fn finish(&self) -> u32 {
        !self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The checksum is ZIP's own: that of "123456789" is CBF43926.
    #[test]
    fn the_checksum_is_zips() {
        let mut crc = Crc32::new();
        crc.add(b"1234");
        crc.add(b"56789");
        assert_eq!(crc.finish(), 0xcbf4_3926);
    }
}
