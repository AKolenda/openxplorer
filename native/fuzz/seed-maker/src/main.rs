// SPDX-License-Identifier: AGPL-3.0-only
//! `ox-fuzz-seed-maker zst IN OUT` compresses IN with Zstandard;
//! `ox-fuzz-seed-maker 7z OUT solid|plain|lzma|bzip2 NAME=TEXT…` writes a
//! 7z with one file per NAME=TEXT (a NAME ending in `/` is a folder).

use std::io::Cursor;

use sevenz_rust2::encoder_options::Lzma2Options;
use sevenz_rust2::{ArchiveEntry, ArchiveWriter, EncoderConfiguration, EncoderMethod, SourceReader};

fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    match arguments.first().map(String::as_str) {
        Some("zst") => {
            let plain = std::fs::read(&arguments[1]).expect("the input");
            let packed = ruzstd::encoding::compress_to_vec(
                plain.as_slice(),
                ruzstd::encoding::CompressionLevel::Fastest,
            );
            std::fs::write(&arguments[2], packed).expect("the output");
        }
        Some("7z") => {
            let bytes = seven_zip(&arguments[2], &arguments[3..]);
            std::fs::write(&arguments[1], bytes).expect("the output");
        }
        _ => panic!("usage: zst IN OUT | 7z OUT MODE NAME=TEXT…"),
    }
}

fn seven_zip(mode: &str, members: &[String]) -> Vec<u8> {
    let mut writer = ArchiveWriter::new(Cursor::new(Vec::new())).expect("a 7z writer");
    let method: EncoderConfiguration = match mode {
        "lzma" => EncoderConfiguration::new(EncoderMethod::LZMA),
        "bzip2" => EncoderConfiguration::new(EncoderMethod::BZIP2),
        "copy" => EncoderConfiguration::new(EncoderMethod::COPY),
        _ => Lzma2Options::from_level(1).into(),
    };
    writer.set_content_methods(vec![method]);
    writer.set_encrypt_header(false);
    let mut files = Vec::new();
    for member in members {
        let (name, text) = member.split_once('=').unwrap_or((member, ""));
        if let Some(folder) = name.strip_suffix('/') {
            let entry = ArchiveEntry::new_directory(folder);
            writer.push_archive_entry::<&[u8]>(entry, None).expect("a folder");
        } else {
            files.push((name.to_owned(), text.as_bytes().repeat(8)));
        }
    }
    if mode == "plain" || mode == "copy" {
        for (name, data) in files {
            let entry = ArchiveEntry::new_file(&name);
            writer
                .push_archive_entry(entry, Some(data.as_slice()))
                .expect("a file");
        }
    } else {
        let entries = files
            .iter()
            .map(|(name, _)| ArchiveEntry::new_file(name))
            .collect();
        let readers = files
            .into_iter()
            .map(|(_, data)| SourceReader::new(Cursor::new(data)))
            .collect();
        writer.push_archive_entries(entries, readers).expect("the files");
    }
    writer.finish().expect("finished").into_inner()
}
