use rc_zip::{
    fsm::{EntryFsm, FsmResult, ParsedRanges},
    parse::{Entry, LocalFileHeader},
};
use std::io;
use tracing::trace;

pub(crate) struct LocalHeaderReader<'a, R>
where
    R: io::Read,
{
    rd: R,
    fsm: Option<EntryFsm<'a>>,
}

impl<'a, R> LocalHeaderReader<'a, R>
where
    R: io::Read,
{
    pub(crate) fn new(entry: &Entry, rd: R, parsed_ranges: &'a mut ParsedRanges) -> Self {
        Self {
            rd,
            fsm: Some(EntryFsm::new(
                Some(entry.clone()),
                None,
                Some(parsed_ranges),
            )),
        }
    }

    pub fn run<'b>(&'b mut self, buf: &mut [u8]) -> io::Result<Option<LocalFileHeader<'b>>> {
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
                        return Ok(self.fsm.as_ref().unwrap().local_header_entry().to_owned());
                    } else if filled_bytes > 0 || outcome.bytes_read > 0 {
                        // progress was made, keep reading
                        continue;
                    } else {
                        return Err(io::Error::new(
                            io::ErrorKind::Other,
                            "entry reader: no progress",
                        ));
                    }
                }
                FsmResult::Done((_, opt_local_header)) => {
                    // neat!
                    return Ok(opt_local_header);
                }
            }
        }
    }
}
