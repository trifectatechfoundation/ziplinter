use std::{rc::Rc, sync::Mutex};

use super::{FsmResult, ParsedRanges};
use crate::{
    encoding::Encoding,
    error::{Error, FormatError},
    parse::{
        Archive, CentralDirectoryFileHeader, EndOfCentralDirectory, EndOfCentralDirectory64Locator,
        EndOfCentralDirectory64Record, EndOfCentralDirectoryRecord, Entry, Located,
    },
};

use ownable::traits::{IntoOwned, ToOwned};
use tracing::trace;
use winnow::{
    error::ErrMode,
    stream::{AsBytes, Offset},
    Parser, Partial,
};

/// [ArchiveFsm] parses a valid zip archive into an [Archive]. In particular, this struct finds
/// an end of central directory record, parses the entire central directory, detects text encoding,
/// and normalizes metadata.
///
/// The loop is as follows:
///
///   * Call [Self::wants_read] to check if more data is needed.
///   * If it returns `Some(offset)`, read the file at that offset
///     into [Self::space] and then call [Self::fill] with
///     the number of bytes read.
///   * Call [Self::process] to process the data.
///   * If it returns [FsmResult::Continue], loop back to the first step.
///
/// Look at the integration tests or
/// [rc-zip-sync](https://crates.io/crates/rc-zip-sync) for concrete examples.
pub struct ArchiveFsm {
    /// Size of the entire zip file
    size: u64,

    /// Current stage: finding the eocd, reading the eocd, reading the eocd64
    /// locator, reading the eocd64, or reading the central directory
    state: State,

    /// Buffer for reading data from the file
    buffer: Buffer,

    /// The ranges that have been parsed while reading the central directory
    parsed_ranges: Rc<Mutex<ParsedRanges>>,
}

#[derive(Default)]
enum State {
    /// Finding and reading the end of central directory record
    ReadEocd {
        /// size of the haystack in which we're looking for the end of central
        /// directory record.
        /// this may be less than 65KiB if the file is smaller than that.
        haystack_size: u64,
    },

    /// Reading the zip64 end of central directory record.
    ReadEocd64Locator {
        eocdr: Located<EndOfCentralDirectoryRecord<'static>>,
    },

    /// Reading the zip64 end of central directory record.
    ReadEocd64 {
        eocdr64_offset: u64,
        eocdr: Located<EndOfCentralDirectoryRecord<'static>>,
    },

    /// Reading all headers from the central directory
    ReadCentralDirectory {
        eocd: EndOfCentralDirectory<'static>,
        directory_headers: Vec<CentralDirectoryFileHeader<'static>>,
        current_header_offset: u64,
    },

    #[default]
    Transitioning,
}

impl ArchiveFsm {
    /// Create a new archive reader with a specified file size.
    pub fn new(size: u64) -> Self {
        // just keep looking for the EndOfCentralDirectory. This is not very efficient, but that's
        // not a priority for our usecase.
        let haystack_size: u64 = size;
        let buffer = Buffer::with_capacity(size as usize);

        Self {
            size,
            buffer,
            state: State::ReadEocd { haystack_size },
            parsed_ranges: Rc::new(Mutex::new(ParsedRanges::new())),
        }
    }

    /// If this returns `Some(offset)`, the caller should read data from
    /// `offset` into [Self::space] — without forgetting to call
    /// [Self::fill] with the number of bytes written.
    pub fn wants_read(&self) -> Option<u64> {
        use State as S;
        match self.state {
            S::ReadEocd { haystack_size } => {
                Some(self.buffer.read_offset(self.size - haystack_size))
            }
            S::ReadEocd64Locator { ref eocdr } => {
                let length = EndOfCentralDirectory64Locator::LENGTH as u64;
                Some(self.buffer.read_offset(eocdr.offset - length))
            }
            S::ReadEocd64 { eocdr64_offset, .. } => Some(self.buffer.read_offset(eocdr64_offset)),
            S::ReadCentralDirectory { ref eocd, .. } => {
                Some(self.buffer.read_offset(eocd.directory_offset()))
            }
            S::Transitioning => unreachable!(),
        }
    }

    /// Process buffered data
    ///
    /// Errors returned from this function are caused by invalid zip archives,
    /// unsupported format quirks, or implementation bugs - never I/O errors.
    ///
    /// A result of [FsmResult::Continue] gives back ownership of the state
    /// machine and indicates the I/O loop should continue, starting with
    /// [Self::wants_read].
    ///
    /// A result of [FsmResult::Done] consumes the state machine and returns
    /// a fully-parsed [Archive].
    pub fn process(mut self) -> Result<FsmResult<Self, Archive>, Error> {
        use State as S;
        match self.state {
            S::ReadEocd { haystack_size } => {
                if self.buffer.read_bytes() < haystack_size {
                    // read the entire haystack before we can continue
                    return Ok(FsmResult::Continue(self));
                }

                let res = {
                    let haystack = &self.buffer.data()[..haystack_size as usize];
                    EndOfCentralDirectoryRecord::find_in_block(haystack)
                };
                match res {
                    None => Err(FormatError::DirectoryEndSignatureNotFound.into()),
                    Some(eocdr) => {
                        trace!(
                            ?eocdr,
                            size = self.size,
                            "ReadEocd | found end of central directory record"
                        );
                        let mut eocdr = eocdr.into_owned();
                        self.buffer.reset();
                        eocdr.offset += self.size - haystack_size;

                        self.parsed_ranges.try_lock().unwrap().insert_offset_length(
                            eocdr.offset,
                            eocdr.inner.len() as u64,
                            "end of central directory record",
                            None,
                        );

                        if eocdr.offset < EndOfCentralDirectory64Locator::LENGTH as u64 {
                            // no room for an EOCD64 locator, definitely not a zip64 file
                            trace!(
                                offset = eocdr.offset,
                                eocd64locator_length = EndOfCentralDirectory64Locator::LENGTH,
                                "no room for an EOCD64 locator, definitely not a zip64 file"
                            );
                            transition!(self.state => (S::ReadEocd { .. }) {
                                let eocd = EndOfCentralDirectory::new(self.size, eocdr, None)?;
                                let current_header_offset = eocd.directory_offset();
                                S::ReadCentralDirectory {
                                    eocd,
                                    directory_headers: vec![],
                                    current_header_offset,
                                }
                            });
                            Ok(FsmResult::Continue(self))
                        } else {
                            trace!("ReadEocd | transition to ReadEocd64Locator");
                            self.buffer.reset();
                            transition!(self.state => (S::ReadEocd { .. }) {
                                S::ReadEocd64Locator { eocdr }
                            });
                            Ok(FsmResult::Continue(self))
                        }
                    }
                }
            }
            S::ReadEocd64Locator { .. } => {
                let input = Partial::new(self.buffer.data());
                match EndOfCentralDirectory64Locator::parser.parse_peek(input) {
                    Err(ErrMode::Incomplete(_)) => {
                        // need more data
                        Ok(FsmResult::Continue(self))
                    }
                    Err(ErrMode::Backtrack(_)) | Err(ErrMode::Cut(_)) => {
                        // we don't have a zip64 end of central directory locator - that's ok!
                        trace!("ReadEocd64Locator | no zip64 end of central directory locator");
                        trace!(
                            "ReadEocd64Locator | data we got: {:02x?}",
                            self.buffer.data()
                        );
                        self.buffer.reset();
                        transition!(self.state => (S::ReadEocd64Locator { eocdr }) {
                            let eocd = EndOfCentralDirectory::new(self.size, eocdr, None)?;
                            let current_header_offset = eocd.directory_offset();
                            S::ReadCentralDirectory {
                                eocd,
                                directory_headers: vec![],
                                current_header_offset,
                            }
                        });
                        Ok(FsmResult::Continue(self))
                    }
                    Ok((_, locator)) => {
                        trace!(
                            ?locator,
                            "ReadEocd64Locator | found zip64 end of central directory locator"
                        );
                        self.buffer.reset();
                        transition!(self.state => (S::ReadEocd64Locator { eocdr }) {
                            let length = EndOfCentralDirectory64Locator::LENGTH as u64;
                            self.parsed_ranges.try_lock().unwrap().insert_offset_length(
                                eocdr.offset - length,
                                length,
                                "zip64 end of central directory locator",
                                None,
                            );

                            S::ReadEocd64 {
                                eocdr64_offset: locator.directory_offset,
                                eocdr,
                            }
                        });
                        Ok(FsmResult::Continue(self))
                    }
                }
            }
            S::ReadEocd64 { .. } => {
                let input = Partial::new(self.buffer.data());
                match EndOfCentralDirectory64Record::parser.parse_peek(input) {
                    Err(ErrMode::Incomplete(_)) => {
                        // need more data
                        Ok(FsmResult::Continue(self))
                    }
                    Err(ErrMode::Backtrack(_)) | Err(ErrMode::Cut(_)) => {
                        // at this point, we really expected to have a zip64 end
                        // of central directory record, so, we want to propagate
                        // that error.
                        Err(FormatError::Directory64EndRecordInvalid.into())
                    }
                    Ok((_, eocdr64)) => {
                        self.buffer.reset();
                        transition!(self.state => (S::ReadEocd64 { eocdr, eocdr64_offset }) {
                            self.parsed_ranges.try_lock().unwrap().insert_offset_length(
                                eocdr64_offset, eocdr64.len() as u64, "zip64 end of central directory record", None
                            );
                            let eocd = EndOfCentralDirectory::new(
                                self.size,
                                eocdr,
                                Some(Located{offset:eocdr64_offset,inner:eocdr64})
                            )?;
                            let current_header_offset = eocd.directory_offset();
                            S::ReadCentralDirectory {
                                eocd,
                                directory_headers:vec![],
                                current_header_offset,
                            }
                        });
                        Ok(FsmResult::Continue(self))
                    }
                }
            }
            S::ReadCentralDirectory {
                ref eocd,
                ref mut directory_headers,
                mut current_header_offset,
            } => {
                trace!(
                    "ReadCentralDirectory | process(), available: {}",
                    self.buffer.available_data()
                );
                let mut valid_consumed = 0;
                let mut input = Partial::new(self.buffer.data());
                trace!(
                    initial_offset = input.as_bytes().offset_from(&self.buffer.data()),
                    initial_len = input.len(),
                    "initial offset & len"
                );
                'read_headers: while !input.is_empty() {
                    match CentralDirectoryFileHeader::parser.parse_next(&mut input) {
                        Ok(dh) => {
                            valid_consumed = input.as_bytes().offset_from(&self.buffer.data());
                            trace!(
                                input_empty_now = input.is_empty(),
                                offset = valid_consumed,
                                len = input.len(),
                                "ReadCentralDirectory | parsed directory header"
                            );

                            let entry = dh.as_entry(Encoding::Utf8, eocd.global_offset as u64);
                            let current_header_end =
                                eocd.directory_offset() + valid_consumed as u64;
                            self.parsed_ranges.try_lock().unwrap().insert_range(
                                current_header_offset..current_header_end,
                                "central directory header",
                                entry.map(|e| e.name).ok(),
                            );

                            current_header_offset += valid_consumed as u64;
                            directory_headers.push(dh.into_owned());
                        }
                        Err(ErrMode::Incomplete(_needed)) => {
                            // need more data to read the full header
                            trace!("ReadCentralDirectory | incomplete!");
                            break 'read_headers;
                        }
                        Err(ErrMode::Backtrack(err)) | Err(ErrMode::Cut(err)) => {
                            // this is the normal end condition when reading
                            // the central directory (due to 65536-entries non-zip64 files)
                            // let's just check a few numbers first.

                            // only compare 16 bits here
                            let expected_records = directory_headers.len() as u16;
                            let actual_records = eocd.directory_records() as u16;

                            if expected_records != actual_records {
                                tracing::trace!(
                                    "error while reading central records: we read {} records, but EOCD announced {}. the last failed with: {err:?} (display: {err}). at that point, input had length {}",
                                    expected_records,
                                    actual_records,
                                    input.len()
                                );

                                // if we read the wrong number of directory entries,
                                // error out.
                                return Err(FormatError::InvalidCentralRecord {
                                    expected: expected_records,
                                    actual: actual_records,
                                }
                                .into());
                            }

                            let mut detectorng = chardetng::EncodingDetector::new();
                            let mut all_utf8 = true;
                            let mut had_suspicious_chars_for_cp437 = false;

                            {
                                let max_feed: usize = 4096;
                                let mut total_fed: usize = 0;
                                let mut feed = |slice: &[u8]| {
                                    detectorng.feed(slice, false);
                                    for b in slice {
                                        if (0xB0..=0xDF).contains(b) {
                                            // those are, like, box drawing characters
                                            had_suspicious_chars_for_cp437 = true;
                                        }
                                    }

                                    total_fed += slice.len();
                                    total_fed < max_feed
                                };

                                'recognize_encoding: for fh in
                                    directory_headers.iter().filter(|fh| fh.is_non_utf8())
                                {
                                    all_utf8 = false;
                                    if !feed(&fh.name[..]) || !feed(&fh.comment[..]) {
                                        break 'recognize_encoding;
                                    }
                                }
                            }

                            let encoding = {
                                if all_utf8 {
                                    Encoding::Utf8
                                } else {
                                    let encoding = detectorng.guess(None, true);
                                    if encoding == encoding_rs::SHIFT_JIS {
                                        // well hold on, sometimes Codepage 437 is detected as
                                        // Shift-JIS by chardetng. If we have any characters
                                        // that aren't valid DOS file names, then okay it's probably
                                        // Shift-JIS. Otherwise, assume it's CP437.
                                        if had_suspicious_chars_for_cp437 {
                                            Encoding::ShiftJis
                                        } else {
                                            Encoding::Cp437
                                        }
                                    } else if encoding == encoding_rs::UTF_8 {
                                        Encoding::Utf8
                                    } else {
                                        Encoding::Cp437
                                    }
                                }
                            };

                            let global_offset = eocd.global_offset as u64;
                            let entries: Result<Vec<Entry>, Error> = directory_headers
                                .iter()
                                .map(|x| x.as_entry(encoding, global_offset))
                                .collect();
                            let entries = entries?;

                            let comment = encoding.decode(eocd.comment())?;

                            return Ok(FsmResult::Done(Archive {
                                eocd: eocd.to_owned(),
                                directory_headers: directory_headers.to_owned(),
                                size: self.size,
                                comment,
                                entries,
                                encoding,
                                parsed_ranges: self.parsed_ranges,
                            }));
                        }
                    }
                }
                let consumed = valid_consumed;
                tracing::trace!(%consumed, "ReadCentralDirectory total consumed");
                self.buffer.consume(consumed);

                // need more data
                Ok(FsmResult::Continue(self))
            }
            S::Transitioning => unreachable!(),
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
}

/// A wrapper around [oval::Buffer] that keeps track of how many bytes we've read since
/// initialization or the last reset.
pub(crate) struct Buffer {
    pub(crate) buffer: oval::Buffer,
    pub(crate) read_bytes: u64,
}

impl Buffer {
    /// creates a new buffer with the specified capacity
    pub(crate) fn with_capacity(size: usize) -> Self {
        Self {
            buffer: oval::Buffer::with_capacity(size),
            read_bytes: 0,
        }
    }

    /// resets the buffer (so that data() returns an empty slice,
    /// and space() returns the full capacity), along with th e
    /// read bytes counter.
    pub(crate) fn reset(&mut self) {
        self.read_bytes = 0;
        self.buffer.reset();
    }

    /// returns the number of read bytes since the last reset
    #[inline]
    pub(crate) fn read_bytes(&self) -> u64 {
        self.read_bytes
    }

    /// returns a slice with all the available data
    #[inline]
    pub(crate) fn data(&self) -> &[u8] {
        self.buffer.data()
    }

    /// returns how much data can be read from the buffer
    #[inline]
    pub(crate) fn available_data(&self) -> usize {
        self.buffer.available_data()
    }

    /// returns how much free space is available to write to
    #[inline]
    pub fn available_space(&self) -> usize {
        self.buffer.available_space()
    }

    /// returns a mutable slice with all the available space to
    /// write to
    #[inline]
    pub(crate) fn space(&mut self) -> &mut [u8] {
        self.buffer.space()
    }

    /// moves the data at the beginning of the buffer
    ///
    /// if the position was more than 0, it is now 0
    #[inline]
    pub fn shift(&mut self) {
        self.buffer.shift()
    }

    /// after having written data to the buffer, use this function
    /// to indicate how many bytes were written
    ///
    /// if there is not enough available space, this function can call
    /// `shift()` to move the remaining data to the beginning of the
    /// buffer
    #[inline]
    pub(crate) fn fill(&mut self, count: usize) -> usize {
        let n = self.buffer.fill(count);
        self.read_bytes += n as u64;
        n
    }

    /// advances the position tracker
    ///
    /// if the position gets past the buffer's half,
    /// this will call `shift()` to move the remaining data
    /// to the beginning of the buffer
    #[inline]
    pub(crate) fn consume(&mut self, size: usize) {
        self.buffer.consume(size);
    }

    /// adds already-read bytes to the given offset. this is useful in
    /// [ArchiveFsm], when we read records at fixed offsets within the file,
    /// that possibly take several reads to fully parse.
    pub(crate) fn read_offset(&self, offset: u64) -> u64 {
        self.read_bytes + offset
    }
}
