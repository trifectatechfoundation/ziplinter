use std::{cmp, rc::Rc, sync::Mutex};

use oval::Buffer;
use tracing::trace;
use winnow::{
    error::ErrMode,
    stream::{AsBytes, Offset},
    Parser, Partial,
};

mod store_dec;

#[cfg(feature = "deflate")]
mod deflate_dec;

#[cfg(feature = "deflate64")]
mod deflate64_dec;

#[cfg(feature = "bzip2")]
mod bzip2_dec;

#[cfg(feature = "lzma")]
mod lzma_dec;

mod aex_dec;
pub use aex_dec::AexData;

#[cfg(feature = "zstd")]
mod zstd_dec;

use crate::{
    error::{Error, FormatError, UnsupportedError},
    parse::{DataDescriptorRecord, Entry, LocalFileHeader, Method},
};

use super::{FsmResult, ParsedRanges};

struct EntryReadMetrics {
    uncompressed_size: u64,
    crc32: u32,
}

#[derive(Default)]
enum State {
    ReadLocalHeader,

    ReadData {
        /// Whether the entry has a data descriptor
        has_data_descriptor: bool,

        /// Whether the entry is zip64 (because its compressed size or uncompressed size is u32::MAX)
        is_zip64: bool,

        /// Amount of bytes we've fed to the decompressor
        compressed_bytes: u64,

        /// Amount of bytes the decompressor has produced
        uncompressed_bytes: u64,

        /// CRC32 hash of the decompressed data
        hasher: crc32fast::Hasher,

        /// The decompression method we're using
        decompressor: AnyDecompressor,

        /// Start of data in file
        data_start: u64,
    },

    ReadDataDescriptor {
        /// Whether the entry is zip64 (because its compressed size or uncompressed size is u32::MAX)
        is_zip64: bool,

        /// Size we've decompressed + crc32 hash we've computed
        metrics: EntryReadMetrics,

        /// Start of data descriptor in file
        data_descriptor_start: u64,
    },

    Validate {
        /// Size we've decompressed + crc32 hash we've computed
        metrics: EntryReadMetrics,

        /// The data descriptor for this entry, if any
        descriptor: Option<DataDescriptorRecord>,
    },

    #[default]
    Transition,
}

/// A state machine that can parse a zip entry
pub struct EntryFsm {
    state: State,
    entry: Option<Entry>,
    local_header: Option<LocalFileHeader<'static>>,
    buffer: Buffer,
    parsed_ranges: Option<Rc<Mutex<ParsedRanges>>>,
    aex_data: Option<AexData>,
}

impl EntryFsm {
    /// Create a new state machine for decompressing a zip entry
    pub fn new(
        entry: Option<Entry>,
        buffer: Option<Buffer>,
        parsed_ranges: Option<Rc<Mutex<ParsedRanges>>>,
    ) -> Self {
        const BUF_CAPACITY: usize = 256 * 1024;

        Self {
            state: State::ReadLocalHeader,
            entry,
            local_header: None,
            buffer: match buffer {
                Some(buffer) => {
                    assert!(buffer.capacity() >= BUF_CAPACITY, "buffer too small");
                    buffer
                }
                None => Buffer::with_capacity(BUF_CAPACITY),
            },
            parsed_ranges,
            aex_data: None,
        }
    }

    /// If this returns true, the caller should read data from into
    /// [Self::space] — without forgetting to call [Self::fill] with the number
    /// of bytes written.
    pub fn wants_read(&self) -> bool {
        match self.state {
            State::ReadLocalHeader => true,
            State::ReadData { .. } => {
                // we want to read if we have space
                self.buffer.available_space() > 0
            }
            State::ReadDataDescriptor { .. } => true,
            State::Validate { .. } => false,
            State::Transition => unreachable!(),
        }
    }

    /// Like `process`, but only processes the header. If this returns
    /// `Ok(None)`, the caller should read more data and call this function
    /// again.
    pub fn process_till_header(&mut self) -> Result<Option<&Entry>, Error> {
        match &self.state {
            State::ReadLocalHeader => {
                self.internal_process_local_header()?;
            }
            _ => {
                // already good
            }
        }

        // this will be non-nil if we've parsed the local header, otherwise,
        Ok(self.entry.as_ref())
    }

    fn internal_process_local_header(&mut self) -> Result<bool, Error> {
        assert!(
            matches!(self.state, State::ReadLocalHeader),
            "internal_process_local_header called in wrong state",
        );

        let mut input = Partial::new(self.buffer.data());
        match LocalFileHeader::parser.parse_next(&mut input) {
            Ok(header) => {
                let consumed = input.as_bytes().offset_from(&self.buffer.data());
                tracing::trace!(local_file_header = ?header, consumed, "parsed local file header");
                let decompressor = AnyDecompressor::new(header.method, self.entry.as_ref())?;

                if self.entry.is_none() {
                    self.entry = Some(header.as_entry()?);
                }

                let entry = self
                    .entry
                    .as_ref()
                    .expect("should always be Some at this point");
                let start = entry.header_offset;
                let length = consumed as u64;

                self.state = State::ReadData {
                    is_zip64: header.compressed_size == u32::MAX
                        || header.uncompressed_size == u32::MAX,
                    has_data_descriptor: header.has_data_descriptor(),
                    compressed_bytes: 0,
                    uncompressed_bytes: 0,
                    hasher: crc32fast::Hasher::new(),
                    decompressor,
                    data_start: start + length,
                };

                if let Some(parsed_ranges) = &mut self.parsed_ranges {
                    parsed_ranges.try_lock().unwrap().insert_offset_length(
                        start,
                        length,
                        "local file header",
                        Some(entry.name.clone()),
                    );
                }

                self.local_header = Some(header.to_owned());

                self.buffer.consume(consumed);
                Ok(true)
            }
            Err(ErrMode::Incomplete(_)) => Ok(false),
            Err(_e) => Err(Error::Format(FormatError::InvalidLocalHeader)),
        }
    }

    /// Process the input and write the output to the given buffer
    ///
    /// This function will return `FsmResult::Continue` if it needs more input
    /// to continue, or if it needs more space to write to. It will return
    /// `FsmResult::Done` when all the input has been decompressed and all
    /// the output has been written.
    ///
    /// Also, after writing all the output, process will read the data
    /// descriptor (if any), and make sur the CRC32 hash and the uncompressed
    /// size match the expected values.
    pub fn process(
        mut self,
        out: &mut [u8],
    ) -> Result<
        FsmResult<(Self, DecompressOutcome), (Buffer, Option<LocalFileHeader>, Option<AexData>)>,
        Error,
    > {
        tracing::trace!(
            state = match &self.state {
                State::ReadLocalHeader => "ReadLocalHeader",
                State::ReadData { .. } => "ReadData",
                State::ReadDataDescriptor { .. } => "ReadDataDescriptor",
                State::Validate { .. } => "Validate",
                State::Transition => "Transition",
            },
            "process"
        );

        use State as S;
        'process_state: loop {
            return match &mut self.state {
                S::ReadLocalHeader => {
                    if self.internal_process_local_header()? {
                        // the local header was completed, let's keep going
                        continue 'process_state;
                    } else {
                        // no buffer were touched, the local header wasn't complete
                        let outcome = DecompressOutcome {
                            bytes_read: 0,
                            bytes_written: 0,
                        };
                        Ok(FsmResult::Continue((self, outcome)))
                    }
                }
                S::ReadData {
                    compressed_bytes,
                    uncompressed_bytes,
                    hasher,
                    decompressor,
                    data_start,
                    ..
                } => {
                    let in_buf = self.buffer.data();
                    let entry = self.entry.as_ref().unwrap();

                    // do we have more input to feed to the decompressor?
                    // if so, don't give it an empty read
                    if in_buf.is_empty() && *compressed_bytes < entry.compressed_size {
                        return Ok(FsmResult::Continue((self, Default::default())));
                    }

                    // don't feed the decompressor bytes beyond the entry's compressed size
                    let in_buf_max_len = cmp::min(
                        in_buf.len(),
                        entry.compressed_size as usize - *compressed_bytes as usize,
                    );
                    let in_buf = &in_buf[..in_buf_max_len];
                    let bytes_fed_this_turn = in_buf.len();

                    let fed_bytes_after_this = *compressed_bytes + in_buf.len() as u64;
                    let has_more_input = if fed_bytes_after_this == entry.compressed_size as _ {
                        HasMoreInput::No
                    } else {
                        HasMoreInput::Yes
                    };

                    trace!(
                        compressed_bytes = *compressed_bytes,
                        uncompressed_bytes = *uncompressed_bytes,
                        fed_bytes_after_this,
                        in_buf_len = in_buf.len(),
                        ?has_more_input,
                        "decompressing"
                    );

                    let outcome = decompressor.decompress(in_buf, out, has_more_input)?;
                    self.buffer.consume(outcome.bytes_read);
                    *compressed_bytes += outcome.bytes_read as u64;
                    trace!(
                        compressed_bytes = *compressed_bytes,
                        uncompressed_bytes = *uncompressed_bytes,
                        entry_compressed_size = %entry.compressed_size,
                        ?outcome,
                        "decompressed"
                    );

                    if outcome.bytes_written == 0 && *compressed_bytes == entry.compressed_size {
                        trace!("eof and no bytes written, we're done");

                        let file_data_end = *data_start + *compressed_bytes;
                        if let Some(parsed_ranges) = &mut self.parsed_ranges {
                            parsed_ranges.try_lock().unwrap().insert_range(
                                *data_start..file_data_end,
                                "file data",
                                Some(entry.name.clone()),
                            );
                        }

                        if let AnyDecompressor::Aex(aex_dec) = decompressor {
                            self.aex_data = aex_dec.take_aex_data();
                        }

                        // we're done, let's read the data descriptor (if there's one)
                        transition!(self.state => (S::ReadData {  has_data_descriptor, is_zip64, uncompressed_bytes, hasher, .. }) {
                            let metrics = EntryReadMetrics {
                                uncompressed_size: uncompressed_bytes,
                                crc32: hasher.finalize(),
                            };

                            if has_data_descriptor {
                                trace!("transitioning to ReadDataDescriptor");
                                S::ReadDataDescriptor { metrics, is_zip64, data_descriptor_start: file_data_end }
                            } else {
                                trace!("transitioning to Validate");
                                S::Validate { metrics, descriptor: None }
                            }
                        });
                        return self.process(out);
                    } else if outcome.bytes_written == 0 && outcome.bytes_read == 0 {
                        if bytes_fed_this_turn == 0 {
                            return Err(Error::IO(std::io::Error::new(
                                std::io::ErrorKind::UnexpectedEof,
                                "decompressor made no progress: this is probably an rc-zip bug",
                            )));
                        } else {
                            // ok fine, continue
                        }
                    }

                    // write the decompressed data to the hasher
                    hasher.update(&out[..outcome.bytes_written]);
                    // update the number of bytes we've decompressed
                    *uncompressed_bytes += outcome.bytes_written as u64;

                    trace!(
                        compressed_bytes = *compressed_bytes,
                        uncompressed_bytes = *uncompressed_bytes,
                        "updated hasher"
                    );

                    Ok(FsmResult::Continue((self, outcome)))
                }
                S::ReadDataDescriptor {
                    is_zip64,
                    data_descriptor_start,
                    ..
                } => {
                    let mut input = Partial::new(self.buffer.data());

                    match DataDescriptorRecord::mk_parser(*is_zip64).parse_next(&mut input) {
                        Ok(descriptor) => {
                            let consumed = input.as_bytes().offset_from(&self.buffer.data());
                            self.buffer.consume(consumed);
                            trace!("data descriptor = {:#?}", descriptor);

                            if let Some(parsed_ranges) = &mut self.parsed_ranges {
                                parsed_ranges.try_lock().unwrap().insert_offset_length(
                                    *data_descriptor_start,
                                    consumed as u64,
                                    "data descriptor",
                                    Some(self.entry.as_ref().unwrap().name.clone()),
                                );
                            }

                            transition!(self.state => (S::ReadDataDescriptor { metrics, .. }) {
                                S::Validate { metrics, descriptor: Some(descriptor) }
                            });
                            self.process(out)
                        }
                        Err(ErrMode::Incomplete(_)) => {
                            Ok(FsmResult::Continue((self, Default::default())))
                        }
                        Err(_e) => Err(Error::Format(FormatError::InvalidDataDescriptor)),
                    }
                }
                S::Validate {
                    metrics,
                    descriptor,
                } => {
                    let entry = self.entry.as_ref().unwrap();

                    let expected_crc32 = if entry.crc32 != 0 {
                        entry.crc32
                    } else if let Some(descriptor) = descriptor.as_ref() {
                        descriptor.crc32
                    } else {
                        0
                    };

                    // When AE-X encryption is used, we can't access the file's data
                    // Since the data is compressed before it is encrypted, the file size of the encrypted data won't match in size
                    // so we skip this validation check
                    if entry.uncompressed_size != metrics.uncompressed_size && entry.aex.is_none() {
                        return Err(Error::Format(FormatError::WrongSize {
                            expected: entry.uncompressed_size,
                            actual: metrics.uncompressed_size,
                        }));
                    }

                    if expected_crc32 != 0 && expected_crc32 != metrics.crc32 && entry.aex.is_none()
                    {
                        return Err(Error::Format(FormatError::WrongChecksum {
                            expected: expected_crc32,
                            actual: metrics.crc32,
                        }));
                    }

                    Ok(FsmResult::Done((
                        self.buffer,
                        self.local_header,
                        self.aex_data,
                    )))
                }
                S::Transition => {
                    unreachable!("the state machine should never be in the transition state")
                }
            };
        }
    }

    /// Returns a mutable slice with all the available space to write to.
    ///
    /// After writing to this, call [Self::fill] with the number of bytes written.
    #[inline]
    pub fn space(&mut self) -> &mut [u8] {
        if self.buffer.available_space() == 0 {
            self.buffer.shift();
        }
        self.buffer.space()
    }

    /// After having written data to [Self::space], call this to indicate how
    /// many bytes were written.
    #[inline]
    pub fn fill(&mut self, count: usize) -> usize {
        self.buffer.fill(count)
    }

    pub fn local_header_entry(&self) -> &Option<LocalFileHeader> {
        &self.local_header
    }
}

enum AnyDecompressor {
    Store(store_dec::StoreDec),
    #[cfg(feature = "deflate")]
    Deflate(Box<deflate_dec::DeflateDec>),
    #[cfg(feature = "deflate64")]
    Deflate64(Box<deflate64_dec::Deflate64Dec>),
    #[cfg(feature = "bzip2")]
    Bzip2(bzip2_dec::Bzip2Dec),
    #[cfg(feature = "lzma")]
    Lzma(Box<lzma_dec::LzmaDec>),
    #[cfg(feature = "zstd")]
    Zstd(zstd_dec::ZstdDec),
    Aex(aex_dec::AexDec),
}

/// Outcome of [EntryFsm::process]
#[derive(Default, Debug)]
pub struct DecompressOutcome {
    /// Number of bytes read from input
    pub bytes_read: usize,

    /// Number of bytes written to output
    pub bytes_written: usize,
}

/// Returns whether there's more input to be fed to the decompressor
#[derive(Debug)]
pub enum HasMoreInput {
    Yes,
    No,
}

trait Decompressor {
    fn decompress(
        &mut self,
        in_buf: &[u8],
        out: &mut [u8],
        has_more_input: HasMoreInput,
    ) -> Result<DecompressOutcome, Error>;
}

impl AnyDecompressor {
    fn new(method: Method, entry: Option<&Entry>) -> Result<Self, Error> {
        let dec = match method {
            Method::Store => Self::Store(Default::default()),

            #[cfg(feature = "deflate")]
            Method::Deflate => Self::Deflate(Default::default()),
            #[cfg(not(feature = "deflate"))]
            Method::Deflate => {
                let err = Error::Unsupported(UnsupportedError::MethodNotEnabled(method));
                return Err(err);
            }

            #[cfg(feature = "deflate64")]
            Method::Deflate64 => Self::Deflate64(Default::default()),
            #[cfg(not(feature = "deflate64"))]
            Method::Deflate64 => {
                let err = Error::Unsupported(UnsupportedError::MethodNotEnabled(method));
                return Err(err);
            }

            #[cfg(feature = "bzip2")]
            Method::Bzip2 => Self::Bzip2(Default::default()),
            #[cfg(not(feature = "bzip2"))]
            Method::Bzip2 => {
                let err = Error::Unsupported(UnsupportedError::MethodNotEnabled(method));
                return Err(err);
            }

            #[cfg(feature = "lzma")]
            Method::Lzma => Self::Lzma(Box::new(lzma_dec::LzmaDec::new(
                entry.map(|e| e.uncompressed_size),
            ))),
            #[cfg(not(feature = "lzma"))]
            Method::Lzma => {
                let err = Error::Unsupported(UnsupportedError::MethodNotEnabled(method));
                return Err(err);
            }

            #[cfg(feature = "zstd")]
            Method::Zstd => Self::Zstd(zstd_dec::ZstdDec::new()?),
            #[cfg(not(feature = "zstd"))]
            Method::Zstd => {
                let err = Error::Unsupported(UnsupportedError::MethodNotEnabled(method));
                return Err(err);
            }

            Method::Aex => match entry {
                Some(Entry { aex: Some(aex), .. }) => Self::Aex(aex_dec::AexDec::new(*aex)),
                _ => panic!(),
            },

            _ => {
                let err = Error::Unsupported(UnsupportedError::MethodNotSupported(method));
                return Err(err);
            }
        };
        Ok(dec)
    }
}

impl Decompressor for AnyDecompressor {
    #[inline]
    fn decompress(
        &mut self,
        in_buf: &[u8],
        out: &mut [u8],
        has_more_input: HasMoreInput,
    ) -> Result<DecompressOutcome, Error> {
        // forward to the appropriate decompressor
        match self {
            Self::Store(dec) => dec.decompress(in_buf, out, has_more_input),
            #[cfg(feature = "deflate")]
            Self::Deflate(dec) => dec.decompress(in_buf, out, has_more_input),
            #[cfg(feature = "deflate64")]
            Self::Deflate64(dec) => dec.decompress(in_buf, out, has_more_input),
            #[cfg(feature = "bzip2")]
            Self::Bzip2(dec) => dec.decompress(in_buf, out, has_more_input),
            #[cfg(feature = "lzma")]
            Self::Lzma(dec) => dec.decompress(in_buf, out, has_more_input),
            #[cfg(feature = "zstd")]
            Self::Zstd(dec) => dec.decompress(in_buf, out, has_more_input),
            Self::Aex(dec) => dec.decompress(in_buf, out, has_more_input),
        }
    }
}
