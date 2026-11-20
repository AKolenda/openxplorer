// SPDX-License-Identifier: AGPL-3.0-only
//! Reading and checking the headers of a 7z archive (`7zFormat.txt` of
//! 7-Zip), before anything else looks at them.
//!
//! The headers are read here record by record, in exactly the order the
//! `sevenz-rust2` reader reads them, and every count and size is checked
//! against the bytes that hold it and against the limits of the built-in
//! reader (ARC-005): a few forged bytes could otherwise ask that reader to
//! set aside more memory than the computer has. Only headers that pass
//! are handed to it, to decode the data; the listing comes from here.

use super::super::ArchiveError;

/// ARC-005: the most files a 7z may hold, as for a ZIP or a TAR.
pub(super) const MAX_FILES: usize = 100_000;
/// ARC-005: the largest header read, packed or not, as for a ZIP's
/// central directory.
pub(super) const MAX_HEADER_BYTES: u64 = 32 * 1024 * 1024;
/// The most coders in one block; 7-Zip writes at most four.
const MAX_CODERS: usize = 32;
/// The most streams of one coder; BCJ2, the widest, has four.
const MAX_CODER_STREAMS: u64 = 32;
/// The longest coder properties; LZMA's are five bytes, AES's at most 34.
const MAX_PROPERTIES: usize = 256;

/// The start of every 7z archive.
pub(crate) const SIGNATURE: [u8; 6] = [b'7', b'z', 0xbc, 0xaf, 0x27, 0x1c];
/// The bytes before the packed data: signature, version and start header.
pub(super) const SIGNATURE_HEADER_BYTES: u64 = 32;

// Property ids of `7zFormat.txt`.
const K_END: u8 = 0x00;
const K_HEADER: u8 = 0x01;
const K_ARCHIVE_PROPERTIES: u8 = 0x02;
const K_ADDITIONAL_STREAMS_INFO: u8 = 0x03;
const K_MAIN_STREAMS_INFO: u8 = 0x04;
const K_FILES_INFO: u8 = 0x05;
const K_PACK_INFO: u8 = 0x06;
const K_UNPACK_INFO: u8 = 0x07;
const K_SUB_STREAMS_INFO: u8 = 0x08;
const K_SIZE: u8 = 0x09;
const K_CRC: u8 = 0x0a;
const K_FOLDER: u8 = 0x0b;
const K_CODERS_UNPACK_SIZE: u8 = 0x0c;
const K_NUM_UNPACK_STREAM: u8 = 0x0d;
const K_EMPTY_STREAM: u8 = 0x0e;
const K_EMPTY_FILE: u8 = 0x0f;
const K_ANTI: u8 = 0x10;
const K_NAME: u8 = 0x11;
const K_C_TIME: u8 = 0x12;
const K_A_TIME: u8 = 0x13;
const K_M_TIME: u8 = 0x14;
const K_WIN_ATTRIBUTES: u8 = 0x15;
pub(super) const K_ENCODED_HEADER: u8 = 0x17;
const K_START_POS: u8 = 0x18;

// Coder ids the built-in reader decodes.
const COPY: &[u8] = &[0x00];
const DELTA: &[u8] = &[0x03];
pub(super) const LZMA: &[u8] = &[0x03, 0x01, 0x01];
pub(super) const LZMA2: &[u8] = &[0x21];
const FILTERS: [&[u8]; 8] = [
    &[0x03, 0x03, 0x01, 0x03], // BCJ x86
    &[0x03, 0x03, 0x02, 0x05], // PowerPC
    &[0x03, 0x03, 0x04, 0x01], // IA-64
    &[0x03, 0x03, 0x05, 0x01], // ARM
    &[0x03, 0x03, 0x07, 0x01], // ARM Thumb
    &[0x03, 0x03, 0x08, 0x05], // SPARC
    &[0x0a],                   // ARM64
    &[0x0b],                   // RISC-V
];
/// `BZip2`, decoded with the `bzip2` crate.
const BZIP2: &[u8] = &[0x04, 0x02, 0x02];
/// AES-256 with a SHA-256 key, 7-Zip's only encryption.
pub(super) const AES: &[u8] = &[0x06, 0xf1, 0x07, 0x01];

/// Why the built-in reader cannot decode a block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum BlockProblem {
    /// It is encrypted.
    Encrypted,
    /// A coder is one the reader lacks, or links streams in a way it
    /// does not read.
    Method,
    /// A dictionary is larger than the reader allows.
    Memory,
}

impl BlockProblem {
    /// The error opening a member of such a block reports.
    pub(super) fn error(self) -> ArchiveError {
        match self {
            Self::Encrypted => ArchiveError::SevenZipEncryption,
            Self::Method => ArchiveError::SevenZipMethod,
            Self::Memory => ArchiveError::DecoderMemoryLimit,
        }
    }
}

/// One coder of a block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Coder {
    pub(super) method: Vec<u8>,
    pub(super) properties: Vec<u8>,
    /// How many streams it reads and writes.
    pub(super) inputs: u64,
    pub(super) outputs: u64,
}

/// One block (a "folder" in `7zFormat.txt`): packed streams decoded
/// together into the data of one file or, in a solid archive, of many.
#[derive(Debug, Clone)]
pub(super) struct Block {
    pub(super) coders: Vec<Coder>,
    pub(super) unpack_size: u64,
    pub(super) crc: Option<u32>,
    /// The bytes it is packed into.
    pub(super) packed_size: u64,
    /// Where its packed data starts, from the start of the file.
    pub(super) packed_offset: u64,
    /// How many files' data it holds.
    pub(super) sub_streams: usize,
    pub(super) problem: Option<BlockProblem>,
}

/// One file or folder of the archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct FileRecord {
    pub(super) name: String,
    pub(super) has_stream: bool,
    pub(super) is_directory: bool,
    pub(super) is_anti: bool,
    pub(super) size: u64,
    /// Hundreds of nanoseconds since 1601, as Windows counts.
    pub(super) modified: Option<u64>,
    pub(super) attributes: Option<u32>,
    /// The block holding its data.
    pub(super) block: Option<usize>,
}

/// The checked headers of a 7z archive.
#[derive(Debug, Clone)]
pub(super) struct Headers {
    pub(super) files: Vec<FileRecord>,
    pub(super) blocks: Vec<Block>,
    /// Where an empty substreams record (`kSubStreamsInfo`, `kEnd`) goes
    /// in the header when the main streams info leaves it out, so that
    /// the `sevenz-rust2` reader, which needs it, reads the same defaults.
    pub(super) missing_sub_streams_at: Option<usize>,
}

/// Where the last header is and its checksum, as the start header says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct NextHeader {
    /// From the end of the start header.
    pub(super) offset: u64,
    pub(super) size: u64,
    pub(super) crc: u32,
}

/// Reads the 32 bytes at the start of a 7z archive of `length` bytes.
///
/// # Errors
///
/// [`ArchiveError::DamagedSevenZip`] for a wrong signature, version or
/// checksum, or a last header outside the file;
/// [`ArchiveError::TooManyMembers`] for one over [`MAX_HEADER_BYTES`].
pub(super) fn next_header(bytes: &[u8; 32], length: u64) -> Result<NextHeader, ArchiveError> {
    if bytes[..6] != SIGNATURE || bytes[6] != 0 {
        return Err(ArchiveError::DamagedSevenZip);
    }
    let crc = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
    // An unfinished archive, whose start header 7-Zip never wrote, is
    // refused: the end of it is not looked for.
    if crc != crc32fast::hash(&bytes[12..32]) {
        return Err(ArchiveError::DamagedSevenZip);
    }
    let field = |start: usize| {
        let mut value = [0u8; 8];
        value.copy_from_slice(&bytes[start..start + 8]);
        u64::from_le_bytes(value)
    };
    let header = NextHeader {
        offset: field(12),
        size: field(20),
        crc: u32::from_le_bytes([bytes[28], bytes[29], bytes[30], bytes[31]]),
    };
    let end = SIGNATURE_HEADER_BYTES
        .checked_add(header.offset)
        .and_then(|start| start.checked_add(header.size))
        .ok_or(ArchiveError::DamagedSevenZip)?;
    if header.size == 0 || end > length {
        return Err(ArchiveError::DamagedSevenZip);
    }
    if header.size > MAX_HEADER_BYTES {
        return Err(ArchiveError::TooManyMembers);
    }
    Ok(header)
}

/// The packed header described by an encoded header (`kEncodedHeader`
/// and its streams info): one block, read from `packed_offset`.
#[derive(Debug, Clone)]
pub(super) struct EncodedHeader {
    pub(super) block: Block,
}

/// Reads an encoded header, which follows its `kEncodedHeader` byte, for
/// an archive of `length` bytes.
///
/// # Errors
///
/// [`ArchiveError::DamagedSevenZip`] for anything malformed,
/// [`ArchiveError::TooManyMembers`] for a header that unpacks to more than
/// [`MAX_HEADER_BYTES`], or the block's problem.
pub(super) fn encoded_header(bytes: &[u8], length: u64) -> Result<EncodedHeader, ArchiveError> {
    let mut reader = Reader::new(bytes);
    let streams = reader.streams_info()?;
    let mut blocks = place_blocks(streams, length)?;
    if blocks.len() != 1 {
        return Err(ArchiveError::DamagedSevenZip);
    }
    let block = blocks.remove(0);
    if block.unpack_size > MAX_HEADER_BYTES {
        return Err(ArchiveError::TooManyMembers);
    }
    if let Some(problem) = block.problem {
        return Err(problem.error());
    }
    Ok(EncodedHeader { block })
}

/// Reads a plain header (`kHeader` and what follows) of an archive of
/// `length` bytes.
///
/// # Errors
///
/// [`ArchiveError::DamagedSevenZip`] for anything malformed or
/// inconsistent, or [`ArchiveError::TooManyMembers`].
pub(super) fn plain_header(bytes: &[u8], length: u64) -> Result<Headers, ArchiveError> {
    let mut reader = Reader::new(bytes);
    if reader.byte()? != K_HEADER {
        return Err(ArchiveError::DamagedSevenZip);
    }
    let mut id = reader.byte()?;
    if id == K_ARCHIVE_PROPERTIES {
        loop {
            let property = reader.byte()?;
            if property == K_END {
                break;
            }
            let size = reader.size()?;
            reader.skip(size)?;
        }
        id = reader.byte()?;
    }
    if id == K_ADDITIONAL_STREAMS_INFO {
        return Err(ArchiveError::DamagedSevenZip);
    }
    let mut streams = Streams::default();
    if id == K_MAIN_STREAMS_INFO {
        streams = reader.streams_info()?;
        id = reader.byte()?;
    }
    let mut files = Vec::new();
    if id == K_FILES_INFO {
        files = reader.files_info()?;
        id = reader.byte()?;
    }
    if id != K_END {
        return Err(ArchiveError::DamagedSevenZip);
    }
    let sub_sizes = streams.sub_sizes.clone();
    let missing_sub_streams_at = streams.missing_sub_streams_at;
    let blocks = place_blocks(streams, length)?;
    assign_blocks(&mut files, &blocks, sub_sizes.as_deref())?;
    Ok(Headers {
        files,
        blocks,
        missing_sub_streams_at,
    })
}

/// The streams info as read, before it is checked against the file.
#[derive(Debug, Default)]
struct Streams {
    pack_position: u64,
    pack_sizes: Vec<u64>,
    blocks: Vec<RawBlock>,
    /// The size of each file's data, in block order; `None` without a
    /// substreams record.
    sub_sizes: Option<Vec<u64>>,
    /// Where the left-out substreams record would have been.
    missing_sub_streams_at: Option<usize>,
}

/// A block as read.
#[derive(Debug, Default)]
struct RawBlock {
    coders: Vec<Coder>,
    total_out: usize,
    bound_out: Vec<u64>,
    /// The bind pairs: an input stream fed by an output stream.
    bind_pairs: Vec<(u64, u64)>,
    /// The input streams read from packed data, in order.
    packed: Vec<u64>,
    packed_streams: usize,
    unpack_sizes: Vec<u64>,
    crc: Option<u32>,
    sub_streams: usize,
}

impl RawBlock {
    /// The size of the block's final output: the one output stream no
    /// bind pair takes, as `Block::get_unpack_size` finds it.
    fn unpack_size(&self) -> u64 {
        (0..self.total_out)
            .rev()
            .find(|index| !self.bound_out.contains(&(*index as u64)))
            .and_then(|index| self.unpack_sizes.get(index).copied())
            .unwrap_or(0)
    }
}

/// Places every block on its packed data and checks that the data lies
/// inside the file.
fn place_blocks(streams: Streams, length: u64) -> Result<Vec<Block>, ArchiveError> {
    let mut next_pack = 0usize;
    let mut offset = SIGNATURE_HEADER_BYTES
        .checked_add(streams.pack_position)
        .ok_or(ArchiveError::DamagedSevenZip)?;
    let mut pack_offsets = Vec::with_capacity(streams.pack_sizes.len());
    for size in &streams.pack_sizes {
        pack_offsets.push(offset);
        offset = offset.checked_add(*size).ok_or(ArchiveError::DamagedSevenZip)?;
    }
    if offset > length {
        return Err(ArchiveError::DamagedSevenZip);
    }
    let mut blocks = Vec::with_capacity(streams.blocks.len());
    for raw in streams.blocks {
        let first = next_pack;
        next_pack = next_pack
            .checked_add(raw.packed_streams)
            .ok_or(ArchiveError::DamagedSevenZip)?;
        if raw.packed_streams == 0 || next_pack > streams.pack_sizes.len() {
            return Err(ArchiveError::DamagedSevenZip);
        }
        let packed_size = streams.pack_sizes[first..next_pack]
            .iter()
            .try_fold(0u64, |sum, size| sum.checked_add(*size))
            .ok_or(ArchiveError::DamagedSevenZip)?;
        let problem = block_problem(&raw);
        blocks.push(Block {
            unpack_size: raw.unpack_size(),
            crc: raw.crc,
            packed_size,
            packed_offset: pack_offsets[first],
            sub_streams: raw.sub_streams,
            coders: raw.coders,
            problem,
        });
    }
    Ok(blocks)
}

/// Why the built-in reader cannot decode `block`, if it cannot.
fn block_problem(block: &RawBlock) -> Option<BlockProblem> {
    if block.coders.iter().any(|coder| coder.method == AES) {
        return Some(BlockProblem::Encrypted);
    }
    if !is_tree(block) {
        return Some(BlockProblem::Method);
    }
    for coder in &block.coders {
        let method = coder.method.as_slice();
        let dictionary = if method == LZMA {
            lzma_dictionary(&coder.properties)
        } else if method == LZMA2 {
            lzma2_dictionary(&coder.properties)
        } else if [COPY, DELTA, BCJ2, BZIP2].contains(&method) || FILTERS.contains(&method) {
            continue;
        } else {
            return Some(BlockProblem::Method);
        };
        match dictionary {
            None => return Some(BlockProblem::Method),
            Some(size) if size > u64::from(crate::archive::MAX_DECODER_MEMORY_KIB) * 1024 => {
                return Some(BlockProblem::Memory);
            }
            Some(_) => {}
        }
    }
    None
}

/// BCJ2, the one coder with more than one input the reader decodes.
const BCJ2: &[u8] = &[0x03, 0x03, 0x01, 0x1b];

/// Whether the coders of `block` form the shapes 7-Zip writes and the
/// `sevenz-rust2` reader decodes: each coder writing one stream, every
/// stream used once, without loops, reaching every coder from the final
/// one; that final coder reads one stream, or is BCJ2 reading four. The
/// crate follows the links without such checks, so a loop in a forged
/// header would never end.
fn is_tree(block: &RawBlock) -> bool {
    if block.coders.iter().any(|coder| coder.outputs != 1) {
        return false;
    }
    let mut first_input = Vec::with_capacity(block.coders.len());
    let mut next_input = 0u64;
    for coder in &block.coders {
        first_input.push(next_input);
        next_input += coder.inputs;
    }
    let inputs: Vec<u64> = block.bind_pairs.iter().map(|pair| pair.0).collect();
    let outputs: Vec<u64> = block.bind_pairs.iter().map(|pair| pair.1).collect();
    let is_distinct = |values: &[u64]| {
        let mut sorted = values.to_vec();
        sorted.sort_unstable();
        sorted.windows(2).all(|pair| pair[0] != pair[1])
    };
    if !is_distinct(&inputs) || !is_distinct(&outputs) || !is_distinct(&block.packed) {
        return false;
    }
    if block.packed.iter().any(|input| inputs.contains(input)) {
        return false;
    }
    // Each coder writes one stream, so stream `n` is coder `n`'s.
    let Some(main) = (0..block.coders.len()).find(|index| !outputs.contains(&(*index as u64))) else {
        return false;
    };
    let main_coder = &block.coders[main];
    let is_main_fine = main_coder.inputs == 1 || (main_coder.method == BCJ2 && main_coder.inputs == 4);
    if !is_main_fine {
        return false;
    }
    let mut visited = vec![false; block.coders.len()];
    let mut waiting = vec![main];
    while let Some(index) = waiting.pop() {
        match visited.get_mut(index) {
            Some(seen) if !*seen => *seen = true,
            _ => return false,
        }
        if index != main && block.coders[index].inputs != 1 {
            return false;
        }
        let start = first_input[index];
        for input in start..start + block.coders[index].inputs {
            if let Some(pair) = block.bind_pairs.iter().find(|pair| pair.0 == input) {
                waiting.push(usize::try_from(pair.1).unwrap_or(usize::MAX));
            } else if !block.packed.contains(&input) {
                return false;
            }
        }
    }
    visited.iter().all(|seen| *seen)
}

/// The dictionary size in LZMA's five property bytes.
pub(super) fn lzma_dictionary(properties: &[u8]) -> Option<u64> {
    let bytes: [u8; 4] = properties.get(1..5)?.try_into().ok()?;
    // lc + lp + pb, as one byte: at most (4 * 5 + 4) * 9 + 8.
    (properties[0] < 9 * 5 * 5).then_some(u64::from(u32::from_le_bytes(bytes)))
}

/// The dictionary size LZMA2's property byte stands for.
pub(super) fn lzma2_dictionary(properties: &[u8]) -> Option<u64> {
    let property = *properties.first()?;
    match property {
        40 => Some(u64::from(u32::MAX)),
        0..=39 => Some((2 | u64::from(property & 1)) << (property / 2 + 11)),
        _ => None,
    }
}

/// Gives every file its block, as `calculate_stream_map` does, and every
/// file with data its size.
fn assign_blocks(
    files: &mut [FileRecord],
    blocks: &[Block],
    sub_sizes: Option<&[u64]>,
) -> Result<(), ArchiveError> {
    let stream_count = files.iter().filter(|file| file.has_stream).count();
    let sizes = match sub_sizes {
        Some(sizes) => sizes,
        None if stream_count == 0 => &[],
        None => return Err(ArchiveError::DamagedSevenZip),
    };
    // Every packed file stream belongs to one file, as 7-Zip requires.
    if stream_count != sizes.len() {
        return Err(ArchiveError::DamagedSevenZip);
    }
    let mut next_stream = 0;
    let mut next_block = 0;
    let mut in_block = 0;
    for file in files.iter_mut() {
        if !file.has_stream && in_block == 0 {
            continue;
        }
        if in_block == 0 {
            while next_block < blocks.len() && blocks[next_block].sub_streams == 0 {
                next_block += 1;
            }
            if next_block >= blocks.len() {
                return Err(ArchiveError::DamagedSevenZip);
            }
        }
        file.block = Some(next_block);
        if !file.has_stream {
            continue;
        }
        file.size = sizes[next_stream];
        next_stream += 1;
        in_block += 1;
        if in_block >= blocks[next_block].sub_streams {
            next_block += 1;
            in_block = 0;
        }
    }
    Ok(())
}

/// Reads the records of a header from its bytes, refusing any count the
/// bytes left could not hold.
struct Reader<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    fn remaining(&self) -> usize {
        self.bytes.len() - self.position
    }

    fn take(&mut self, count: usize) -> Result<&'a [u8], ArchiveError> {
        if count > self.remaining() {
            return Err(ArchiveError::DamagedSevenZip);
        }
        let taken = &self.bytes[self.position..self.position + count];
        self.position += count;
        Ok(taken)
    }

    fn skip(&mut self, count: usize) -> Result<(), ArchiveError> {
        self.take(count).map(|_| ())
    }

    fn byte(&mut self) -> Result<u8, ArchiveError> {
        Ok(self.take(1)?[0])
    }

    fn u32(&mut self) -> Result<u32, ArchiveError> {
        let bytes = self.take(4)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn u64(&mut self) -> Result<u64, ArchiveError> {
        let mut value = [0u8; 8];
        value.copy_from_slice(self.take(8)?);
        Ok(u64::from_le_bytes(value))
    }

    /// A 7z number: the leading one bits of the first byte say how many
    /// bytes follow.
    fn number(&mut self) -> Result<u64, ArchiveError> {
        let first = u64::from(self.byte()?);
        let mut mask = 0x80u64;
        let mut value = 0u64;
        for index in 0..8 {
            if first & mask == 0 {
                let high = first & (mask - 1);
                return Ok(value | (high << (8 * index)));
            }
            value |= u64::from(self.byte()?) << (8 * index);
            mask >>= 1;
        }
        Ok(value)
    }

    /// A number that counts or sizes something held in the bytes left.
    fn size(&mut self) -> Result<usize, ArchiveError> {
        let value = self.number()?;
        usize::try_from(value)
            .ok()
            .filter(|size| *size <= self.remaining())
            .ok_or(ArchiveError::DamagedSevenZip)
    }

    /// A count of at most `limit` things, each held in at least one bit
    /// of the bytes left.
    fn count(&mut self, limit: usize) -> Result<usize, ArchiveError> {
        let value = self.number()?;
        let count = usize::try_from(value).map_err(|_| ArchiveError::TooManyMembers)?;
        if count > limit {
            return Err(ArchiveError::TooManyMembers);
        }
        if count > self.remaining().saturating_mul(8) {
            return Err(ArchiveError::DamagedSevenZip);
        }
        Ok(count)
    }

    /// `count` bits, the first in the top bit of the first byte.
    fn bits(&mut self, count: usize) -> Result<Vec<bool>, ArchiveError> {
        let bytes = self.take(count.div_ceil(8))?;
        Ok((0..count)
            .map(|index| {
                bytes
                    .get(index / 8)
                    .is_some_and(|byte| byte & (0x80 >> (index % 8)) != 0)
            })
            .collect())
    }

    /// An "all defined" byte, then the bits only when it is 0.
    fn all_or_bits(&mut self, count: usize) -> Result<Vec<bool>, ArchiveError> {
        if self.byte()? != 0 {
            Ok(vec![true; count])
        } else {
            self.bits(count)
        }
    }

    fn streams_info(&mut self) -> Result<Streams, ArchiveError> {
        let mut streams = Streams::default();
        let mut id = self.byte()?;
        if id == K_PACK_INFO {
            self.pack_info(&mut streams)?;
            id = self.byte()?;
        }
        if id == K_UNPACK_INFO {
            self.unpack_info(&mut streams)?;
            id = self.byte()?;
        }
        if id == K_SUB_STREAMS_INFO {
            self.sub_streams_info(&mut streams)?;
            id = self.byte()?;
        } else if !streams.blocks.is_empty() {
            // The record is optional (`7zFormat.txt`): without it each
            // block holds one file of the block's size, with the block's
            // checksum.
            for block in &mut streams.blocks {
                block.sub_streams = 1;
            }
            streams.sub_sizes = Some(streams.blocks.iter().map(RawBlock::unpack_size).collect());
            streams.missing_sub_streams_at = Some(self.position - 1);
        }
        if id != K_END {
            return Err(ArchiveError::DamagedSevenZip);
        }
        Ok(streams)
    }

    fn pack_info(&mut self, streams: &mut Streams) -> Result<(), ArchiveError> {
        streams.pack_position = self.number()?;
        let count = self.count(MAX_FILES)?;
        let mut id = self.byte()?;
        if id == K_SIZE {
            streams.pack_sizes = (0..count).map(|_| self.number()).collect::<Result<_, _>>()?;
            id = self.byte()?;
        }
        if streams.pack_sizes.len() != count {
            return Err(ArchiveError::DamagedSevenZip);
        }
        if id == K_CRC {
            let defined = self.all_or_bits(count)?;
            for is_defined in defined {
                if is_defined {
                    self.u32()?;
                }
            }
            id = self.byte()?;
        }
        if id != K_END {
            return Err(ArchiveError::DamagedSevenZip);
        }
        Ok(())
    }

    fn unpack_info(&mut self, streams: &mut Streams) -> Result<(), ArchiveError> {
        if self.byte()? != K_FOLDER {
            return Err(ArchiveError::DamagedSevenZip);
        }
        let count = self.count(MAX_FILES)?;
        if self.byte()? != 0 {
            return Err(ArchiveError::DamagedSevenZip);
        }
        let mut blocks = Vec::with_capacity(count);
        for _ in 0..count {
            blocks.push(self.block()?);
        }
        if self.byte()? != K_CODERS_UNPACK_SIZE {
            return Err(ArchiveError::DamagedSevenZip);
        }
        for block in &mut blocks {
            block.unpack_sizes = (0..block.total_out)
                .map(|_| self.number())
                .collect::<Result<_, _>>()?;
        }
        let mut id = self.byte()?;
        if id == K_CRC {
            let defined = self.all_or_bits(count)?;
            for (block, is_defined) in blocks.iter_mut().zip(defined) {
                block.crc = if is_defined { Some(self.u32()?) } else { None };
            }
            id = self.byte()?;
        }
        if id != K_END {
            return Err(ArchiveError::DamagedSevenZip);
        }
        streams.blocks = blocks;
        Ok(())
    }

    fn block(&mut self) -> Result<RawBlock, ArchiveError> {
        let coder_count = self.number()?;
        if coder_count == 0 || coder_count > MAX_CODERS as u64 {
            return Err(ArchiveError::DamagedSevenZip);
        }
        let mut block = RawBlock::default();
        let (mut total_in, mut total_out) = (0u64, 0u64);
        for _ in 0..coder_count {
            let flags = self.byte()?;
            let id = self.take(usize::from(flags & 0x0f))?.to_vec();
            let is_simple = flags & 0x10 == 0;
            let (inputs, outputs) = if is_simple {
                (1, 1)
            } else {
                (self.number()?, self.number()?)
            };
            if inputs > MAX_CODER_STREAMS || outputs > MAX_CODER_STREAMS {
                return Err(ArchiveError::DamagedSevenZip);
            }
            total_in += inputs;
            total_out += outputs;
            let mut properties = Vec::new();
            if flags & 0x20 != 0 {
                let size = self.size()?;
                if size > MAX_PROPERTIES {
                    return Err(ArchiveError::DamagedSevenZip);
                }
                properties = self.take(size)?.to_vec();
            }
            // Alternative methods: the reference reader refuses them too.
            if flags & 0x80 != 0 {
                return Err(ArchiveError::DamagedSevenZip);
            }
            block.coders.push(Coder {
                method: id,
                properties,
                inputs,
                outputs,
            });
        }
        if total_out == 0 {
            return Err(ArchiveError::DamagedSevenZip);
        }
        let bind_pairs = total_out - 1;
        let mut bound_in = Vec::new();
        for _ in 0..bind_pairs {
            let input = self.number()?;
            let output = self.number()?;
            if input >= total_in || output >= total_out {
                return Err(ArchiveError::DamagedSevenZip);
            }
            bound_in.push(input);
            block.bound_out.push(output);
            block.bind_pairs.push((input, output));
        }
        if total_in < bind_pairs {
            return Err(ArchiveError::DamagedSevenZip);
        }
        let packed = total_in - bind_pairs;
        if packed == 1 {
            let first_free = (0..total_in)
                .find(|index| !bound_in.contains(index))
                .ok_or(ArchiveError::DamagedSevenZip)?;
            block.packed.push(first_free);
        } else {
            for _ in 0..packed {
                let input = self.number()?;
                if input >= total_in {
                    return Err(ArchiveError::DamagedSevenZip);
                }
                block.packed.push(input);
            }
        }
        block.total_out = usize::try_from(total_out).map_err(|_| ArchiveError::DamagedSevenZip)?;
        block.packed_streams = usize::try_from(packed).map_err(|_| ArchiveError::DamagedSevenZip)?;
        Ok(block)
    }

    fn sub_streams_info(&mut self, streams: &mut Streams) -> Result<(), ArchiveError> {
        for block in &mut streams.blocks {
            block.sub_streams = 1;
        }
        let mut id = self.byte()?;
        let mut total = streams.blocks.len();
        if id == K_NUM_UNPACK_STREAM {
            total = 0;
            for block in &mut streams.blocks {
                block.sub_streams = self.count(MAX_FILES)?;
                total += block.sub_streams;
                if total > MAX_FILES {
                    return Err(ArchiveError::TooManyMembers);
                }
            }
            id = self.byte()?;
        }
        let mut sizes = Vec::with_capacity(total);
        for block in &streams.blocks {
            if block.sub_streams == 0 {
                continue;
            }
            let unpack_size = block.unpack_size();
            let mut sum = 0u64;
            if id == K_SIZE {
                for _ in 1..block.sub_streams {
                    let size = self.number()?;
                    sum = sum.checked_add(size).ok_or(ArchiveError::DamagedSevenZip)?;
                    sizes.push(size);
                }
            } else {
                sizes.extend(std::iter::repeat_n(0, block.sub_streams - 1));
            }
            if sum > unpack_size {
                return Err(ArchiveError::DamagedSevenZip);
            }
            sizes.push(unpack_size - sum);
        }
        if id == K_SIZE {
            id = self.byte()?;
        }
        let digests: usize = streams
            .blocks
            .iter()
            .filter(|block| block.sub_streams != 1 || block.crc.is_none())
            .map(|block| block.sub_streams)
            .sum();
        if id == K_CRC {
            let defined = self.all_or_bits(digests)?;
            for is_defined in defined {
                if is_defined {
                    self.u32()?;
                }
            }
            id = self.byte()?;
        }
        if id != K_END {
            return Err(ArchiveError::DamagedSevenZip);
        }
        streams.sub_sizes = Some(sizes);
        Ok(())
    }

    fn files_info(&mut self) -> Result<Vec<FileRecord>, ArchiveError> {
        let count = self.count(MAX_FILES)?;
        let mut files = vec![
            FileRecord {
                name: String::new(),
                has_stream: true,
                is_directory: false,
                is_anti: false,
                size: 0,
                modified: None,
                attributes: None,
                block: None,
            };
            count
        ];
        let mut empty_stream: Option<Vec<bool>> = None;
        let mut empty_file: Option<Vec<bool>> = None;
        let mut anti: Option<Vec<bool>> = None;
        let mut has_names = false;
        loop {
            let property = self.byte()?;
            if property == K_END {
                break;
            }
            let size = self.size()?;
            match property {
                // The empty-file and anti bits count the empty streams,
                // so the empty streams are named once, before them.
                K_EMPTY_STREAM if empty_stream.is_some() => return Err(ArchiveError::DamagedSevenZip),
                K_EMPTY_STREAM => empty_stream = Some(self.bits(count)?),
                K_EMPTY_FILE | K_ANTI => {
                    let empty_count = empty_stream
                        .as_ref()
                        .ok_or(ArchiveError::DamagedSevenZip)?
                        .iter()
                        .filter(|is_empty| **is_empty)
                        .count();
                    let bits = Some(self.bits(empty_count)?);
                    if property == K_EMPTY_FILE {
                        empty_file = bits;
                    } else {
                        anti = bits;
                    }
                }
                K_NAME => {
                    if self.byte()? != 0 || size == 0 || (size - 1) % 2 != 0 {
                        return Err(ArchiveError::DamagedSevenZip);
                    }
                    let text = self.take(size - 1)?;
                    let names = utf16_names(text)?;
                    if names.len() != count {
                        return Err(ArchiveError::DamagedSevenZip);
                    }
                    for (file, name) in files.iter_mut().zip(names) {
                        file.name = name;
                    }
                    has_names = true;
                }
                K_C_TIME | K_A_TIME | K_M_TIME => {
                    let defined = self.all_or_bits(count)?;
                    if self.byte()? != 0 {
                        return Err(ArchiveError::DamagedSevenZip);
                    }
                    for (file, is_defined) in files.iter_mut().zip(defined) {
                        if is_defined {
                            let time = self.u64()?;
                            if property == K_M_TIME {
                                file.modified = Some(time);
                            }
                        } else if property == K_M_TIME {
                            file.modified = None;
                        }
                    }
                }
                K_WIN_ATTRIBUTES => {
                    let defined = self.all_or_bits(count)?;
                    if self.byte()? != 0 {
                        return Err(ArchiveError::DamagedSevenZip);
                    }
                    for (file, is_defined) in files.iter_mut().zip(defined) {
                        file.attributes = if is_defined { Some(self.u32()?) } else { None };
                    }
                }
                K_START_POS => return Err(ArchiveError::DamagedSevenZip),
                _ => self.skip(size)?,
            }
        }
        if count > 0 && !has_names {
            return Err(ArchiveError::DamagedSevenZip);
        }
        mark_empty_streams(
            &mut files,
            empty_stream.as_ref(),
            empty_file.as_ref(),
            anti.as_ref(),
        )?;
        Ok(files)
    }
}

/// Marks the `files` without a stream of their own: folders, unless the
/// empty-file bits say they are empty files, and anti items. The bits
/// come from the file, so each is read with `get`, never indexed.
fn mark_empty_streams(
    files: &mut [FileRecord],
    empty_stream: Option<&Vec<bool>>,
    empty_file: Option<&Vec<bool>>,
    anti: Option<&Vec<bool>>,
) -> Result<(), ArchiveError> {
    let bit = |bits: Option<&Vec<bool>>, at: usize| {
        bits.map_or(Some(false), |bits| bits.get(at).copied())
            .ok_or(ArchiveError::DamagedSevenZip)
    };
    let mut empty_index = 0;
    for (index, file) in files.iter_mut().enumerate() {
        file.has_stream = !bit(empty_stream, index)?;
        if file.has_stream {
            continue;
        }
        file.is_directory = !bit(empty_file, empty_index)?;
        file.is_anti = bit(anti, empty_index)?;
        empty_index += 1;
    }
    Ok(())
}

/// The names of a `kName` record: UTF-16 little-endian, each ending with
/// a 0, as `NamesReader` splits them.
fn utf16_names(text: &[u8]) -> Result<Vec<String>, ArchiveError> {
    let mut names = Vec::new();
    let mut current = Vec::new();
    for pair in text.chunks_exact(2) {
        let unit = u16::from_le_bytes([pair[0], pair[1]]);
        if unit == 0 {
            names.push(String::from_utf16(&current).map_err(|_| ArchiveError::DamagedSevenZip)?);
            current.clear();
        } else {
            current.push(unit);
        }
    }
    if !current.is_empty() {
        names.push(String::from_utf16(&current).map_err(|_| ArchiveError::DamagedSevenZip)?);
    }
    Ok(names)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_take_as_many_bytes_as_their_first_byte_says() {
        let cases: [(&[u8], u64); 4] = [
            (&[0x7f], 0x7f),
            (&[0x80, 0x80], 0x80),
            (&[0xc1, 0x02, 0x03], 0x01_0302),
            (&[0xff, 1, 2, 3, 4, 5, 6, 7, 8], 0x0807_0605_0403_0201),
        ];
        for (bytes, value) in cases {
            assert_eq!(Reader::new(bytes).number().expect("a number"), value);
        }
    }

    #[test]
    fn dictionaries_are_read_from_lzma_and_lzma2_properties() {
        assert_eq!(lzma2_dictionary(&[0]), Some(4096));
        assert_eq!(lzma2_dictionary(&[1]), Some(6144));
        assert_eq!(lzma2_dictionary(&[16]), Some(1 << 20));
        assert_eq!(lzma2_dictionary(&[40]), Some(u64::from(u32::MAX)));
        assert_eq!(lzma2_dictionary(&[41]), None);
        assert_eq!(lzma_dictionary(&[0x5d, 0, 0, 0, 1]), Some(1 << 24));
        assert_eq!(lzma_dictionary(&[0x5d, 0]), None);
    }

    #[test]
    fn a_count_larger_than_its_bytes_could_hold_is_refused() {
        let mut reader = Reader::new(&[0x40, 0x10, 1, 2]);
        assert!(matches!(
            reader.count(MAX_FILES),
            Err(ArchiveError::DamagedSevenZip)
        ));
        let mut reader = Reader::new(&[0xff, 0, 0, 0, 0, 0, 1, 0, 0]);
        assert!(matches!(
            reader.count(MAX_FILES),
            Err(ArchiveError::TooManyMembers)
        ));
    }

    /// A damaged files record that names its empty streams again after
    /// their empty-file bits, so the bits no longer cover them, is
    /// refused instead of read past the end; so are the records of a
    /// seeded fuzz, each refused or read without a panic.
    #[test]
    fn a_files_record_that_repeats_its_empty_streams_is_refused() {
        let names = [0x11, 9, 0, b'a', 0, 0, 0, b'b', 0, 0, 0];
        let mut damaged = vec![2, K_EMPTY_STREAM, 1, 0b0100_0000, K_EMPTY_FILE, 1, 0x80];
        damaged.extend_from_slice(&[K_EMPTY_STREAM, 1, 0b1100_0000]);
        damaged.extend_from_slice(&names);
        damaged.push(K_END);
        assert!(matches!(
            Reader::new(&damaged).files_info(),
            Err(ArchiveError::DamagedSevenZip)
        ));
        let mut anti = vec![2, K_EMPTY_STREAM, 1, 0b0100_0000, K_ANTI, 1, 0x80];
        anti.extend_from_slice(&[K_EMPTY_STREAM, 1, 0b1100_0000]);
        anti.extend_from_slice(&names);
        anti.push(K_END);
        assert!(Reader::new(&anti).files_info().is_err());
        let mut good = vec![2, K_EMPTY_STREAM, 1, 0b0100_0000, K_EMPTY_FILE, 1, 0x80];
        good.extend_from_slice(&names);
        good.push(K_END);
        let files = Reader::new(&good).files_info().expect("a good record");
        assert!(files[0].has_stream && !files[1].has_stream && !files[1].is_directory);
        let mut seed = 0x1234_5678_u32;
        for _ in 0..20_000 {
            let mut bytes = good.clone();
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            for step in 0..(seed % 6) {
                let at = usize::try_from(seed.rotate_left(step * 5)).unwrap_or(0) % bytes.len();
                bytes[at] = seed.to_le_bytes()[usize::try_from(step % 4).unwrap_or(0)] & 0x1f;
            }
            let _ = Reader::new(&bytes).files_info();
            let _ = Reader::new(&bytes).streams_info();
        }
    }
}
