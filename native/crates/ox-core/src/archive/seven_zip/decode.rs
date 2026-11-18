// SPDX-License-Identifier: AGPL-3.0-only
//! Decoding the data of a 7z block with the `sevenz-rust2` crate.
//!
//! The crate decodes a block by handing each of its files to a callback
//! in turn. Here that runs on a thread of its own, which opens the archive
//! again and sends the files' data over a short channel; the reader takes
//! it a chunk at a time, as it reads a ZIP or TAR member. The thread stops
//! as soon as the reader is dropped or the user cancels.
//!
//! The archive opened again must be the one whose headers were checked
//! (the same length and start header), and the crate is given only the
//! header checked by `header`, placed after the archive's own bytes, never
//! the archive's own: a packed header is not unpacked again, and a header
//! that leaves out the optional substreams record gets an empty one.

use std::io::{self, Read, Seek, SeekFrom};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, RecvTimeoutError, SyncSender};
use std::sync::Arc;
use std::time::Duration;

use sevenz_rust2::{Archive, BlockDecoder, Password};

use super::header::{self, Block, LZMA, LZMA2, SIGNATURE_HEADER_BYTES};
use super::{damaged, Parsed};
use crate::archive::source::{ArchiveOpener, ArchiveStream};
use crate::archive::ArchiveError;
use crate::transfer::Cancellation;

/// The bytes of one message of data.
const CHUNK: usize = 64 * 1024;
/// Chunks waiting for the reader: a little decoding ahead, never more.
const CHANNEL_CHUNKS: usize = 4;
/// How often a reader waiting for data looks at its cancellation.
const POLL: Duration = Duration::from_millis(100);

/// What the decoding thread sends.
enum Message {
    /// The file at this index starts.
    Start(usize),
    /// Its next bytes.
    Data(Vec<u8>),
    /// It has ended, checksum checked.
    End,
    /// Decoding failed.
    Failed(ArchiveError),
}

/// A block being decoded on its own thread.
pub(super) struct BlockStream {
    block: usize,
    /// The first file whose data has not been handed out.
    next_file: usize,
    /// The first file whose data the reader wants; earlier ones are
    /// decoded but not sent.
    wanted: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    receiver: Receiver<Message>,
    cancel: Cancellation,
}

impl Drop for BlockStream {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

impl BlockStream {
    /// Starts decoding block `block` of the archive at `uri`.
    pub(super) fn start(
        opener: Arc<dyn ArchiveOpener>,
        uri: String,
        parsed: Arc<Parsed>,
        block: usize,
        cancel: Cancellation,
    ) -> Self {
        let (sender, receiver) = sync_channel(CHANNEL_CHUNKS);
        let wanted = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let decoding = Decoding {
            sender,
            wanted: Arc::clone(&wanted),
            stop: Arc::clone(&stop),
            cancel: cancel.clone(),
        };
        let spawned = std::thread::Builder::new()
            .name("ox-7z-decoder".to_owned())
            .spawn(move || decoding.run(opener.as_ref(), &uri, &parsed, block));
        // A thread that cannot start drops its sender, so the reader
        // finds the channel closed and reports a failure.
        drop(spawned);
        Self {
            block,
            next_file: 0,
            wanted,
            stop,
            receiver,
            cancel,
        }
    }

    /// The first file whose data has not been handed out.
    pub(super) fn next_file(&self) -> usize {
        self.next_file
    }

    /// Whether this stream can still hand out file `file` of block
    /// `block`: it decodes that block and has not passed the file.
    pub(super) fn can_reach(&self, block: usize, file: usize) -> bool {
        self.block == block && self.next_file <= file
    }

    /// Skips to file `file` and returns its data.
    ///
    /// # Errors
    ///
    /// The decoding's failure, or a cancellation.
    pub(super) fn member(&mut self, file: usize) -> Result<MemberChunks<'_>, ArchiveError> {
        self.wanted.store(file, Ordering::SeqCst);
        loop {
            match self.receive()? {
                Message::Start(index) if index == file => break,
                Message::Start(index) if index > file => return Err(ArchiveError::DamagedSevenZip),
                Message::Start(_) | Message::Data(_) | Message::End => {}
                Message::Failed(error) => return Err(error),
            }
        }
        self.next_file = file + 1;
        Ok(MemberChunks {
            stream: self,
            pending: Vec::new(),
            offset: 0,
            is_finished: false,
        })
    }

    /// The next message, looking at the cancellation while waiting.
    fn receive(&self) -> Result<Message, ArchiveError> {
        loop {
            self.cancel.check()?;
            match self.receiver.recv_timeout(POLL) {
                Ok(message) => return Ok(message),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return Err(ArchiveError::DamagedSevenZip),
            }
        }
    }
}

/// The data of one file, as the decoding thread sends it.
pub(super) struct MemberChunks<'a> {
    stream: &'a mut BlockStream,
    pending: Vec<u8>,
    offset: usize,
    is_finished: bool,
}

impl MemberChunks<'_> {
    /// Reads the next bytes into `buffer`; 0 at the file's end.
    pub(super) fn read(&mut self, buffer: &mut [u8]) -> Result<usize, ArchiveError> {
        while self.offset >= self.pending.len() {
            if self.is_finished {
                return Ok(0);
            }
            match self.stream.receive()? {
                Message::Data(data) => {
                    self.pending = data;
                    self.offset = 0;
                }
                Message::End => self.is_finished = true,
                Message::Failed(error) => return Err(error),
                Message::Start(_) => return Err(ArchiveError::DamagedSevenZip),
            }
        }
        let count = buffer.len().min(self.pending.len() - self.offset);
        buffer[..count].copy_from_slice(&self.pending[self.offset..self.offset + count]);
        self.offset += count;
        Ok(count)
    }
}

/// The decoding thread's side.
struct Decoding {
    sender: SyncSender<Message>,
    wanted: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    cancel: Cancellation,
}

impl Decoding {
    fn run(self, opener: &dyn ArchiveOpener, uri: &str, parsed: &Parsed, block: usize) {
        let result = catch_unwind(AssertUnwindSafe(|| self.decode(opener, uri, parsed, block)));
        let failure = match result {
            Ok(Ok(())) => return,
            Ok(Err(error)) => error,
            // The checked headers keep the crate's reader on its paths; a
            // panic left anyway means damaged data.
            Err(_) => ArchiveError::DamagedSevenZip,
        };
        let _ = self.sender.send(Message::Failed(failure));
    }

    fn decode(
        &self,
        opener: &dyn ArchiveOpener,
        uri: &str,
        parsed: &Parsed,
        block: usize,
    ) -> Result<(), ArchiveError> {
        let mut stream = opener.open(uri, &self.cancel)?;
        // The archive is opened again here: it must still be the one whose
        // headers were checked, or its data would be read under them.
        if !is_same_archive(stream.as_mut(), parsed)? {
            return Err(ArchiveError::SevenZipChanged);
        }
        let mut source: Box<dyn ArchiveStream> = Box::new(WithPlainHeader::new(
            stream,
            parsed.length,
            Arc::clone(&parsed.checked_header),
        ));
        let password = Password::empty();
        let archive = Archive::read(&mut source, &password).map_err(|error| crate_error(&error))?;
        if archive.files.len() != parsed.headers.files.len()
            || archive.blocks.len() != parsed.headers.blocks.len()
        {
            return Err(ArchiveError::DamagedSevenZip);
        }
        let first = *archive
            .stream_map
            .block_first_file_index
            .get(block)
            .ok_or(ArchiveError::DamagedSevenZip)?;
        let mut next = first;
        let mut failure: Option<ArchiveError> = None;
        let decoder = BlockDecoder::new(1, block, &archive, &password, &mut source);
        let outcome = decoder.for_each_entries(&mut |_entry, reader| {
            let file = next;
            next += 1;
            match self.send_file(file, reader) {
                Ok(()) => Ok(true),
                Err(error) => {
                    failure = Some(error);
                    Err(sevenz_rust2::Error::Other("stopped".into()))
                }
            }
        });
        match (outcome, failure) {
            (_, Some(error)) => Err(error),
            (Err(error), None) => Err(crate_error(&error)),
            (Ok(_), None) => Ok(()),
        }
    }

    /// Sends file `file`, read from `reader`: its data only when the
    /// reader wants it, though it is always decoded and checked.
    fn send_file(&self, file: usize, reader: &mut dyn Read) -> Result<(), ArchiveError> {
        self.send(Message::Start(file))?;
        let is_wanted = file >= self.wanted.load(Ordering::SeqCst);
        loop {
            if self.stop.load(Ordering::SeqCst) {
                return Err(ArchiveError::Cancelled);
            }
            self.cancel.check()?;
            let mut chunk = vec![0u8; CHUNK];
            let count = read_full(reader, &mut chunk).map_err(data_error)?;
            if count == 0 {
                break;
            }
            if is_wanted {
                chunk.truncate(count);
                self.send(Message::Data(chunk))?;
            }
        }
        self.send(Message::End)
    }

    fn send(&self, message: Message) -> Result<(), ArchiveError> {
        // The reader is gone: stop quietly.
        self.sender.send(message).map_err(|_| ArchiveError::Cancelled)
    }
}

/// Whether `stream` is the archive `parsed` was read from: the same
/// length and the same first 32 bytes, which hold the checksum of its last
/// header. Any change to the names, sizes or checksums of its files
/// changes that header; damaged data then fails its own checksum.
fn is_same_archive(stream: &mut dyn ArchiveStream, parsed: &Parsed) -> Result<bool, ArchiveError> {
    let length = stream.seek(SeekFrom::End(0))?;
    stream.seek(SeekFrom::Start(0))?;
    let mut start = [0u8; 32];
    if length != parsed.length || stream.read_exact(&mut start).is_err() {
        return Ok(false);
    }
    Ok(start == parsed.start)
}

/// Fills `chunk` as far as `reader` goes; fewer bytes only at its end.
fn read_full(reader: &mut dyn Read, chunk: &mut [u8]) -> io::Result<usize> {
    let mut filled = 0;
    while filled < chunk.len() {
        match reader.read(&mut chunk[filled..]) {
            Ok(0) => break,
            Ok(count) => filled += count,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(filled)
}

/// The error of reading a file's data: a checksum or decoding failure is
/// damage, anything else the source's own error.
fn data_error(error: io::Error) -> ArchiveError {
    let is_from_the_decoder = error
        .get_ref()
        .is_some_and(<dyn std::error::Error + Send + Sync>::is::<sevenz_rust2::Error>);
    if is_from_the_decoder || error.kind() == io::ErrorKind::Other {
        ArchiveError::DamagedSevenZip
    } else {
        damaged(error)
    }
}

/// The app's error for an error of the crate.
fn crate_error(error: &sevenz_rust2::Error) -> ArchiveError {
    use sevenz_rust2::Error;
    match error {
        Error::MaxMemLimited { .. } => ArchiveError::DecoderMemoryLimit,
        Error::UnsupportedCompressionMethod(_) | Error::Unsupported(_) => ArchiveError::SevenZipMethod,
        Error::PasswordRequired | Error::MaybeBadPassword(_) => ArchiveError::SevenZipEncryption,
        _ => ArchiveError::DamagedSevenZip,
    }
}

/// Unpacks the packed header `block` of the archive in `stream`: one LZMA
/// or LZMA2 coder, as 7-Zip packs headers, with its checksum checked.
///
/// # Errors
///
/// [`ArchiveError::SevenZipMethod`] for any other coding,
/// [`ArchiveError::DamagedSevenZip`] for damaged data, or the source's
/// error.
pub(super) fn unpack_header(stream: &mut dyn ArchiveStream, block: &Block) -> Result<Vec<u8>, ArchiveError> {
    let [coder] = block.coders.as_slice() else {
        return Err(ArchiveError::SevenZipMethod);
    };
    stream.seek(SeekFrom::Start(block.packed_offset))?;
    let packed = stream.take(block.packed_size);
    let size = usize::try_from(block.unpack_size).map_err(|_| ArchiveError::DamagedSevenZip)?;
    let mut plain = vec![0u8; size];
    if coder.method == LZMA {
        let dictionary = header::lzma_dictionary(&coder.properties).ok_or(ArchiveError::DamagedSevenZip)?;
        let dictionary = u32::try_from(dictionary).map_err(|_| ArchiveError::DecoderMemoryLimit)?;
        let mut reader = lzma_rust2::LzmaReader::new_with_props(
            packed,
            block.unpack_size,
            coder.properties[0],
            dictionary,
            None,
        )
        .map_err(damaged)?;
        reader.read_exact(&mut plain).map_err(damaged)?;
    } else if coder.method == LZMA2 {
        let dictionary = header::lzma2_dictionary(&coder.properties).ok_or(ArchiveError::DamagedSevenZip)?;
        let dictionary = u32::try_from(dictionary).map_err(|_| ArchiveError::DecoderMemoryLimit)?;
        let mut reader = lzma_rust2::Lzma2Reader::new(packed, dictionary, None);
        reader.read_exact(&mut plain).map_err(damaged)?;
    } else {
        return Err(ArchiveError::SevenZipMethod);
    }
    if block.crc.is_some_and(|crc| crc != crc32fast::hash(&plain)) {
        return Err(ArchiveError::DamagedSevenZip);
    }
    Ok(plain)
}

/// An archive whose header is replaced by the header checked here: a new
/// start header pointing past the archive's own bytes, where the checked
/// header follows. The packed data stays where the headers say it is.
struct WithPlainHeader {
    inner: Box<dyn ArchiveStream>,
    start: [u8; 32],
    length: u64,
    plain: Arc<[u8]>,
    position: u64,
    /// Where `inner` is, when known.
    inner_position: Option<u64>,
}

impl WithPlainHeader {
    fn new(inner: Box<dyn ArchiveStream>, length: u64, plain: Arc<[u8]>) -> Self {
        let mut fields = Vec::with_capacity(20);
        fields.extend_from_slice(&(length - SIGNATURE_HEADER_BYTES).to_le_bytes());
        fields.extend_from_slice(&(plain.len() as u64).to_le_bytes());
        fields.extend_from_slice(&crc32fast::hash(&plain).to_le_bytes());
        let mut start = [0u8; 32];
        start[..6].copy_from_slice(&super::SIGNATURE);
        start[7] = 4;
        start[8..12].copy_from_slice(&crc32fast::hash(&fields).to_le_bytes());
        start[12..].copy_from_slice(&fields);
        Self {
            inner,
            start,
            length,
            plain,
            position: 0,
            inner_position: None,
        }
    }

    fn total(&self) -> u64 {
        self.length + self.plain.len() as u64
    }
}

impl Read for WithPlainHeader {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let count = if self.position < SIGNATURE_HEADER_BYTES {
            let start = usize::try_from(self.position).unwrap_or(0);
            let count = buffer.len().min(32 - start);
            buffer[..count].copy_from_slice(&self.start[start..start + count]);
            count
        } else if self.position < self.length {
            let wanted = usize::try_from(self.length - self.position).unwrap_or(usize::MAX);
            let limit = buffer.len().min(wanted);
            if self.inner_position != Some(self.position) {
                self.inner.seek(SeekFrom::Start(self.position))?;
            }
            self.inner_position = None;
            let count = self.inner.read(&mut buffer[..limit])?;
            self.inner_position = Some(self.position + count as u64);
            count
        } else {
            let start = usize::try_from(self.position - self.length).unwrap_or(usize::MAX);
            let available = self.plain.get(start..).unwrap_or_default();
            let count = buffer.len().min(available.len());
            buffer[..count].copy_from_slice(&available[..count]);
            count
        };
        self.position += count as u64;
        Ok(count)
    }
}

impl Seek for WithPlainHeader {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        let target = match position {
            SeekFrom::Start(offset) => Some(offset),
            SeekFrom::End(offset) => self.total().checked_add_signed(offset),
            SeekFrom::Current(offset) => self.position.checked_add_signed(offset),
        };
        self.position = target.ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput))?;
        Ok(self.position)
    }
}
