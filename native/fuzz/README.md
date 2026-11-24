# Fuzz targets

Fuzz targets for the parsers that read bytes from outside the app: archives,
pictures, media, text previews, desktop entries, the child processes' answers,
D-Bus requests, the root helpers' command lines and the update server's
answers. They run with [cargo-fuzz](https://github.com/rust-fuzz/cargo-fuzz)
(libFuzzer and AddressSanitizer) on a nightly toolchain.

This crate is not a member of the native workspace, so `cargo build`,
`native/tools/check.py`, the packages and the Flatpak never compile it. The
targets reach private parsers through the `fuzzing` features of `ox-core`
and `ox-app`, which are off in every build of the app.

## Running

```bash
rustup toolchain install nightly
cargo install cargo-fuzz
cd native/fuzz
python3 seeds.py
cargo +nightly fuzz build -O
./run.sh archive_list 3600
```

`seeds.py` writes small valid files into `corpus/<target>/`, made by the
script itself (Python's `zipfile` and `tarfile`, Pillow, ffmpeg when it is
installed, and `seed-maker` for 7z and Zstandard). `run.sh TARGET SECONDS`
runs one target with a 2-second timeout and a 1 GiB memory limit, keeping
new inputs in `corpus/` and failures in `artifacts/`; `stats.py` adds up the
executions in `logs/`. With `OX_FUZZ_WORK` set to a folder outside the
checkout, all three scripts keep `corpus/`, `artifacts/` and `logs/` there
instead, so a long run's files stay out of the tree the checks copy.
Archive targets need `-detect_leaks=0` after the seconds: the 7z decoder's
thread may still be running when an input ends.

| Target | What it reads |
|---|---|
| `archive_list` | ZIP, 7z and TAR (plain, gzip, bzip2, XZ, Zstandard): listing, Extract All's check, previews |
| `archive_extract` | Extract All into a new folder, checking nothing lands outside it |
| `archive_fixup` | TARs and 7z with their checksums set first, TARs compressed in the harness |
| `archive_zipgen` | ZIPs written from the input with consistent offsets and checksums |
| `core_text` | Locations, clipboard lists, file dialog requests, mount tables, `user-dirs.dirs`, catalogs, SMB snapshot lists, bookmarks, service menus, names |
| `core_formats` | The root helpers' command lines, mount plans, update releases and addresses, account and share lists, search words, saved sessions |
| `picture_headers` | The app's own picture header readers, fed in pieces, and the decoder and drawer answers |
| `picture_decode` | A picture decoded as the decoder process decodes it |
| `media_details` | Picture headers, EXIF and XMP as Properties reads them, the reader's JSON, Rotate, thumbnailer entries |
| `media_streams` | Video and audio files read with GStreamer as the media reader does (slow; use `-timeout=10`) |
| `text_preview` | The Preview pane's text and rich text readers, desktop links |
| `app_protocols` | The media player's lines, a portal's parent window, typed names, the command line |

A failure found here becomes a regression test next to the parser it is
about, with the minimized input written in the test's code.
