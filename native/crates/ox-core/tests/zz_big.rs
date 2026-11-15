mod archive_support;
use archive_support::*;
use ox_core::archive::*;
use ox_core::transfer::Cancellation;
use std::time::Instant;
#[test]
fn big() {
    let dir = std::path::PathBuf::from("/home/tester/big");
    let c = Cancellation::new();
    let t = Instant::now();
    let r = ZipCompressor::new().compress(
        &CompressionRequest {
            uris: vec![file_uri(&dir.join("Survey.bin"))],
            destination_uri: file_uri(&dir),
            archive_name: "Survey.zip".into(),
        },
        &c,
    );
    println!(
        "compress: {:?} in {:?}",
        r.as_ref().map(|c| c.uri.clone()),
        t.elapsed()
    );
    let out = dir.join("out");
    let _ = std::fs::create_dir(&out);
    let t = Instant::now();
    let mut x = ZipExtractor::new(opener(), gio_factory(), std::sync::Arc::new(GioExtractionOutput));
    let r = x.extract(
        &ExtractionRequest {
            archive_uri: file_uri(&dir.join("Survey.zip")),
            destination_uri: file_uri(&out),
            folder_name: "x".into(),
        },
        &c,
    );
    println!("extract: {:?} in {:?}", r.map(|f| f.summary), t.elapsed());
}
