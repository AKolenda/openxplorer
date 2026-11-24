// SPDX-License-Identifier: AGPL-3.0-only
//! Empty OpenDocument files for the New menu: a text document, a
//! spreadsheet and a presentation, as LibreOffice opens them.
//!
//! An OpenDocument file is a ZIP package (ODF 1.2, part 3): a `mimetype`
//! entry first, stored uncompressed without an extra field, so programs
//! recognize the type from the file's first bytes; then the manifest
//! (`META-INF/manifest.xml`), which lists the other entries, and the
//! document's `content.xml` and `styles.xml`. Every entry is stored, with
//! its sizes and CRC-32 in its local header, and dated 1 January 1980, so
//! the same document always gives the same bytes.

/// Which empty OpenDocument file to make.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum OpenDocumentKind {
    /// `.odt`: one empty paragraph.
    Text,
    /// `.ods`: one sheet, "Sheet1", with one empty cell.
    Spreadsheet,
    /// `.odp`: one empty slide, 28 × 15.75 cm (16:9), LibreOffice
    /// Impress's own size.
    Presentation,
}

impl OpenDocumentKind {
    /// The package's media type, the `mimetype` entry's content.
    pub(super) fn media_type(self) -> &'static str {
        match self {
            OpenDocumentKind::Text => "application/vnd.oasis.opendocument.text",
            OpenDocumentKind::Spreadsheet => "application/vnd.oasis.opendocument.spreadsheet",
            OpenDocumentKind::Presentation => "application/vnd.oasis.opendocument.presentation",
        }
    }

    /// The `office:body` element's content.
    fn body(self) -> &'static str {
        match self {
            OpenDocumentKind::Text => "<office:text><text:p/></office:text>",
            OpenDocumentKind::Spreadsheet => concat!(
                "<office:spreadsheet><table:table table:name=\"Sheet1\"><table:table-column/>",
                "<table:table-row><table:table-cell/></table:table-row></table:table></office:spreadsheet>"
            ),
            OpenDocumentKind::Presentation => concat!(
                "<office:presentation><draw:page draw:name=\"page1\" draw:master-page-name=\"Default\"/>",
                "</office:presentation>"
            ),
        }
    }

    /// The styles: none for text and spreadsheets, which take the
    /// program's defaults; for a presentation, the master page its slide
    /// names, with Impress's 16:9 page.
    fn styles(self) -> &'static str {
        match self {
            OpenDocumentKind::Text | OpenDocumentKind::Spreadsheet => "",
            OpenDocumentKind::Presentation => concat!(
                "<office:automatic-styles><style:page-layout style:name=\"PM1\">",
                "<style:page-layout-properties fo:margin-top=\"0cm\" fo:margin-bottom=\"0cm\" ",
                "fo:margin-left=\"0cm\" fo:margin-right=\"0cm\" fo:page-width=\"28cm\" ",
                "fo:page-height=\"15.75cm\" style:print-orientation=\"landscape\"/>",
                "</style:page-layout></office:automatic-styles><office:master-styles>",
                "<style:master-page style:name=\"Default\" style:page-layout-name=\"PM1\"/>",
                "</office:master-styles>"
            ),
        }
    }
}

/// The XML declaration every part starts with.
const XML_DECLARATION: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n";

/// The namespaces the parts use.
const NAMESPACES: &str = concat!(
    "xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\" ",
    "xmlns:style=\"urn:oasis:names:tc:opendocument:xmlns:style:1.0\" ",
    "xmlns:text=\"urn:oasis:names:tc:opendocument:xmlns:text:1.0\" ",
    "xmlns:table=\"urn:oasis:names:tc:opendocument:xmlns:table:1.0\" ",
    "xmlns:draw=\"urn:oasis:names:tc:opendocument:xmlns:drawing:1.0\" ",
    "xmlns:fo=\"urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0\""
);

/// The ODF version the parts declare.
const ODF_VERSION: &str = "1.2";

/// The empty OpenDocument file of `kind`, whole.
pub(super) fn empty_document(kind: OpenDocumentKind) -> Vec<u8> {
    let media_type = kind.media_type();
    let manifest = format!(
        concat!(
            "{declaration}<manifest:manifest ",
            "xmlns:manifest=\"urn:oasis:names:tc:opendocument:xmlns:manifest:1.0\" ",
            "manifest:version=\"{version}\">",
            "<manifest:file-entry manifest:full-path=\"/\" manifest:version=\"{version}\" ",
            "manifest:media-type=\"{media_type}\"/>",
            "<manifest:file-entry manifest:full-path=\"content.xml\" manifest:media-type=\"text/xml\"/>",
            "<manifest:file-entry manifest:full-path=\"styles.xml\" manifest:media-type=\"text/xml\"/>",
            "</manifest:manifest>\n"
        ),
        declaration = XML_DECLARATION,
        version = ODF_VERSION,
        media_type = media_type,
    );
    let content = format!(
        "{XML_DECLARATION}<office:document-content {NAMESPACES} office:version=\"{ODF_VERSION}\">\
         <office:body>{}</office:body></office:document-content>\n",
        kind.body()
    );
    let styles = format!(
        "{XML_DECLARATION}<office:document-styles {NAMESPACES} office:version=\"{ODF_VERSION}\">{}\
         </office:document-styles>\n",
        kind.styles()
    );
    stored_zip(&[
        ("mimetype", media_type.as_bytes()),
        ("META-INF/manifest.xml", manifest.as_bytes()),
        ("content.xml", content.as_bytes()),
        ("styles.xml", styles.as_bytes()),
    ])
}

/// `PK\x03\x04`: a local file header.
const LOCAL_HEADER_SIGNATURE: u32 = 0x0403_4b50;
/// `PK\x01\x02`: a central directory header.
const CENTRAL_HEADER_SIGNATURE: u32 = 0x0201_4b50;
/// `PK\x05\x06`: the end of the central directory.
const END_SIGNATURE: u32 = 0x0605_4b50;
/// Version 1.0: stored entries only.
const VERSION_STORED: u16 = 10;
/// 1 January 1980, the earliest MS-DOS date, as ZIP stores it.
const DOS_DATE_1980: u16 = (1 << 5) | 1;

/// A ZIP of `entries`, in their order, each stored uncompressed with its
/// CRC-32 and sizes in its local header. The entries are a few kilobytes
/// of fixed text, far below every 16- and 32-bit limit of the format.
pub(crate) fn stored_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut zip = Vec::new();
    let mut directory = Vec::new();
    for &(name, data) in entries {
        let offset = small(zip.len());
        let crc = crc32fast::hash(data);
        for (buffer, signature) in [
            (&mut zip, LOCAL_HEADER_SIGNATURE),
            (&mut directory, CENTRAL_HEADER_SIGNATURE),
        ] {
            push_u32(buffer, signature);
            if signature == CENTRAL_HEADER_SIGNATURE {
                push_u16(buffer, VERSION_STORED);
            }
            push_u16(buffer, VERSION_STORED);
            push_u16(buffer, 0); // flags
            push_u16(buffer, 0); // stored
            push_u16(buffer, 0); // time
            push_u16(buffer, DOS_DATE_1980);
            push_u32(buffer, crc);
            push_u32(buffer, small(data.len()));
            push_u32(buffer, small(data.len()));
            push_u16(buffer, u16::try_from(name.len()).expect("a short name"));
            push_u16(buffer, 0); // no extra field
            if signature == CENTRAL_HEADER_SIGNATURE {
                push_u16(buffer, 0); // no comment
                push_u16(buffer, 0); // disk 0
                push_u16(buffer, 0); // internal attributes
                push_u32(buffer, 0); // external attributes
                push_u32(buffer, offset);
            }
            buffer.extend_from_slice(name.as_bytes());
        }
        zip.extend_from_slice(data);
    }
    let directory_offset = small(zip.len());
    let count = u16::try_from(entries.len()).expect("a few entries");
    zip.extend_from_slice(&directory);
    push_u32(&mut zip, END_SIGNATURE);
    push_u16(&mut zip, 0); // this disk
    push_u16(&mut zip, 0); // the directory's disk
    push_u16(&mut zip, count);
    push_u16(&mut zip, count);
    push_u32(&mut zip, small(directory.len()));
    push_u32(&mut zip, directory_offset);
    push_u16(&mut zip, 0); // no comment
    zip
}

/// `value` as a 32-bit ZIP field; the packages are tiny.
fn small(value: usize) -> u32 {
    u32::try_from(value).expect("an OpenDocument starter is tiny")
}

fn push_u16(buffer: &mut Vec<u8>, value: u16) {
    buffer.extend_from_slice(&value.to_le_bytes());
}

fn push_u32(buffer: &mut Vec<u8>, value: u32) {
    buffer.extend_from_slice(&value.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    const KINDS: [OpenDocumentKind; 3] = [
        OpenDocumentKind::Text,
        OpenDocumentKind::Spreadsheet,
        OpenDocumentKind::Presentation,
    ];

    fn u16_at(bytes: &[u8], at: usize) -> u16 {
        u16::from_le_bytes(bytes[at..at + 2].try_into().expect("two bytes"))
    }

    fn u32_at(bytes: &[u8], at: usize) -> u32 {
        u32::from_le_bytes(bytes[at..at + 4].try_into().expect("four bytes"))
    }

    /// The entries of `zip` as its central directory lists them, each
    /// read from its local header and checked against its CRC-32.
    fn entries(zip: &[u8]) -> Vec<(String, Vec<u8>)> {
        let end = zip.len() - 22;
        assert_eq!(u32_at(zip, end), END_SIGNATURE, "the end record closes the file");
        let count = usize::from(u16_at(zip, end + 10));
        let mut at = u32_at(zip, end + 16) as usize;
        let mut found = Vec::new();
        for _ in 0..count {
            assert_eq!(u32_at(zip, at), CENTRAL_HEADER_SIGNATURE);
            let crc = u32_at(zip, at + 16);
            let size = u32_at(zip, at + 20) as usize;
            let name_length = usize::from(u16_at(zip, at + 28));
            let local = u32_at(zip, at + 42) as usize;
            let name = String::from_utf8(zip[at + 46..at + 46 + name_length].to_vec()).expect("UTF-8");
            assert_eq!(u32_at(zip, local), LOCAL_HEADER_SIGNATURE, "{name}");
            assert_eq!(u16_at(zip, local + 8), 0, "{name} is stored");
            assert_eq!(
                u32_at(zip, local + 14),
                crc,
                "{name}: the local header has the CRC"
            );
            assert_eq!(u16_at(zip, local + 28), 0, "{name}: no extra field");
            let data_at = local + 30 + usize::from(u16_at(zip, local + 26));
            let data = zip[data_at..data_at + size].to_vec();
            assert_eq!(crc32fast::hash(&data), crc, "{name}: its CRC-32");
            found.push((name, data));
            at += 46 + name_length;
        }
        found
    }

    /// Each package starts with its stored `mimetype` entry, so the type
    /// shows at byte 30 as `file` and LibreOffice read it, and lists every
    /// part in its manifest, with matching CRC-32s.
    #[test]
    fn each_document_is_a_well_formed_opendocument_package() {
        for kind in KINDS {
            let package = empty_document(kind);
            let expected = format!("mimetype{}", kind.media_type());

            assert_eq!(&package[30..30 + expected.len()], expected.as_bytes(), "{kind:?}");
            let entries = entries(&package);
            let names: Vec<&str> = entries.iter().map(|(name, _)| name.as_str()).collect();
            assert_eq!(
                names,
                ["mimetype", "META-INF/manifest.xml", "content.xml", "styles.xml"],
                "{kind:?}"
            );
            let manifest = String::from_utf8(entries[1].1.clone()).expect("UTF-8");
            assert!(
                manifest.contains(&format!("manifest:media-type=\"{}\"", kind.media_type())),
                "{kind:?}: {manifest}"
            );
            for part in ["content.xml", "styles.xml"] {
                assert!(
                    manifest.contains(&format!("full-path=\"{part}\"")),
                    "{kind:?}: {part}"
                );
            }
            assert_eq!(
                empty_document(kind),
                package,
                "{kind:?}: the same bytes every time"
            );
        }
    }
}
