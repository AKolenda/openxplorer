mod archive_support;
use archive_support::*;
use ox_core::archive::*;
use ox_core::transfer::Cancellation;
use std::time::Instant;

#[test]
fn probe() {
    let dir = std::path::PathBuf::from(std::env::var("PROBE_DIR").unwrap());
    let c = Cancellation::new();
    for name in [
        "backslash.zip",
        "many.zip",
        "unicode.zip",
        "truncated.zip",
        "corrupt.zip",
        "crcbad.zip",
        "zip64.zip",
    ] {
        let uri = file_uri(&dir.join(name));
        let b = ArchiveBrowser::new(opener(), dir.join("prev"));
        let t = Instant::now();
        let l = b.list(&uri, "", &c);
        match &l {
            Ok(l) => println!(
                "{name}: list ok {} entries hidden {} trunc {} in {:?}: {:?}",
                l.entries.len(),
                l.hidden_unsafe_count,
                l.is_truncated,
                t.elapsed(),
                l.entries
                    .iter()
                    .take(3)
                    .map(|e| e.name.clone())
                    .collect::<Vec<_>>()
            ),
            Err(e) => println!("{name}: list ERR {e}"),
        }
        let out = dir.join(format!("out-{name}"));
        let _ = std::fs::create_dir(&out);
        let mut x = ZipExtractor::new(opener(), gio_factory(), std::sync::Arc::new(GioExtractionOutput));
        let t = Instant::now();
        let r = x.extract(
            &ExtractionRequest {
                archive_uri: uri.clone(),
                destination_uri: file_uri(&out),
                folder_name: "x".into(),
            },
            &c,
        );
        match r {
            Ok(f) => println!(
                "   extract ok {} files in {:?}",
                f.summary.file_count,
                t.elapsed()
            ),
            Err(e) => println!("   extract ERR {e}"),
        }
        let left: Vec<_> = std::fs::read_dir(&out)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        println!("   left in dest: {left:?}");
    }
}
