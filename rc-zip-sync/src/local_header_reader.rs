use rc_zip::{
    fsm::{EntryFsm, FsmResult, ParsedRanges},
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
}

impl<'a, R> LocalHeaderReader<'a, R>
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
        }
    }

    pub(crate) fn get_local_header(&self) -> Option<LocalFileHeader<'_>> {
        self.local_header.to_owned()
    }
}

impl<'a, R> io::Read for LocalHeaderReader<'a, R>
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
                        return Err(io::Error::new(
                            io::ErrorKind::Other,
                            "entry reader: no progress",
                        ));
                    }
                }
                FsmResult::Done((_, local_file_header)) => {
                    if let Some(local_header) = local_file_header {
                        self.local_header = Some(local_header.into_owned());
                    }

                    // neat!
                    return Ok(0);
                }
            }
        }
    }
}
