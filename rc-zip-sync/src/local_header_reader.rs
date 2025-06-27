use rc_zip::{
    fsm::{AexData, EntryFsm, FsmResult, ParsedRanges},
    parse::{Entry, LocalFileHeader},
};
use std::{io, rc::Rc, sync::Mutex};
use tracing::trace;

pub(crate) struct LocalHeaderReader<'a, R>
where
    R: io::Read,
{
    rd: R,
    fsm: Option<EntryFsm>,
    local_header: Option<LocalFileHeader<'a>>,
    aex_data: Option<AexData>,
}

impl<R> LocalHeaderReader<'_, R>
where
    R: io::Read,
{
    pub(crate) fn new(entry: &Entry, rd: R, parsed_ranges: Rc<Mutex<ParsedRanges>>) -> Self {
        Self {
            rd,
            fsm: Some(EntryFsm::new(
                Some(entry.clone()),
                None,
                Some(parsed_ranges),
            )),
            local_header: None,
            aex_data: None,
        }
    }

    pub(crate) fn take_local_header(&mut self) -> Option<LocalFileHeader<'_>> {
        self.local_header.take()
    }

    pub(crate) fn take_aex_data(&mut self) -> Option<AexData> {
        self.aex_data.take()
    }
}

impl<R> io::Read for LocalHeaderReader<'_, R>
where
    R: io::Read,
{
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        loop {
            let mut fsm = match self.fsm.take() {
                Some(fsm) => fsm,
                None => unreachable!(),
            };

            #[allow(clippy::needless_late_init)] // don't tell me what to do
            let filled_bytes;
            if fsm.wants_read() {
                tracing::trace!(space_avail = fsm.space().len(), "fsm wants read");
                let n = self.rd.read(fsm.space())?;
                fsm.fill(n);
                filled_bytes = n;
            } else {
                trace!("fsm does not want read");
                filled_bytes = 0;
            }

            match fsm.process(buf)? {
                FsmResult::Continue((fsm, outcome)) => {
                    self.fsm = Some(fsm);

                    if outcome.bytes_written > 0 {
                        tracing::trace!("wrote {} bytes", outcome.bytes_written);
                        return Ok(outcome.bytes_written);
                    } else if filled_bytes > 0 || outcome.bytes_read > 0 {
                        // progress was made, keep reading
                        continue;
                    } else {
                        return Err(io::Error::other("entry reader: no progress"));
                    }
                }
                FsmResult::Done((_, local_file_header, aex_data)) => {
                    self.local_header = local_file_header.map(|s| s.into_owned());
                    self.aex_data = aex_data;

                    // neat!
                    return Ok(0);
                }
            }
        }
    }
}
