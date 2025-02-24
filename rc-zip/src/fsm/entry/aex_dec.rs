use std::cmp;

use crate::{error::Error, parse::ExtraAexField};

use super::{DecompressOutcome, Decompressor, HasMoreInput};

pub(crate) struct AexDec {
    aex: ExtraAexField,
    position: usize,
}

impl AexDec {
    pub(crate) fn new(aex: ExtraAexField) -> Self {
        Self { aex, position: 0 }
    }
}

impl Decompressor for AexDec {
    fn decompress(
        &mut self,
        in_buf: &[u8],
        out_buf: &mut [u8],
        _has_more_input: HasMoreInput,
    ) -> Result<DecompressOutcome, Error> {
        // https://www.winzip.com/en/support/aes-encryption/#file-format1

        let key_bytes = match self.aex.mode {
            0x1 => 8,
            0x2 => 12,
            0x3 => 16,
            _ => return Err(Error::Format(crate::error::FormatError::InvalidExtraField)),
        };

        if in_buf.len() < key_bytes + 2 {
            self.position += in_buf.len();
            return Ok(DecompressOutcome {
                bytes_read: in_buf.len(),
                bytes_written: 0,
            });
        }

        let (_salt_value, rest) = in_buf.split_at(key_bytes);
        let (_password_verification_value, rest) = rest.split_at(2);
        let (data, _authentication_code) = rest.split_at(rest.len() - 9);

        let bytes_read = cmp::min(in_buf.len(), out_buf.len());
        let len = data.len();
        out_buf[..len].copy_from_slice(&data[..len]);
        Ok(DecompressOutcome {
            bytes_read,
            bytes_written: len,
        })
    }
}
