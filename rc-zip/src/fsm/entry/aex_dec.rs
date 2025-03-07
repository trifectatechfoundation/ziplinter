use std::cmp;

use crate::{error::Error, parse::ExtraAexField};

use super::{DecompressOutcome, Decompressor, HasMoreInput};

pub(crate) struct AexDec {
    aex: ExtraAexField,
    salt_value: Option<Vec<u8>>,
    password_verification_value: Option<Vec<u8>>,
    authentication_code: Option<Vec<u8>>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct AexData {
    salt_value: Vec<u8>,
    password_verification_value: Vec<u8>,
    authentication_code: Vec<u8>,
}

impl AexDec {
    pub(crate) fn new(aex: ExtraAexField) -> Self {
        Self {
            aex,
            salt_value: None,
            password_verification_value: None,
            authentication_code: None,
        }
    }

    pub fn take_aex_data(&mut self) -> Option<AexData> {
        Some(AexData {
            salt_value: self.salt_value.take()?,
            password_verification_value: self.password_verification_value.take()?,
            authentication_code: self.authentication_code.take()?,
        })
    }
}

impl Decompressor for AexDec {
    fn decompress(
        &mut self,
        in_buf: &[u8],
        out_buf: &mut [u8],
        has_more_input: HasMoreInput,
    ) -> Result<DecompressOutcome, Error> {
        // https://www.winzip.com/en/support/aes-encryption/#file-format1

        const PASSWORD_VERIFICATION_SIZE: usize = 2;
        const AUTHENTICATION_CODE_SIZE: usize = 10;

        let salt_size = match self.aex.mode {
            0x1 => 8,
            0x2 => 12,
            0x3 => 16,
            _ => return Err(Error::Format(crate::error::FormatError::InvalidExtraField)),
        };

        if in_buf.len() < salt_size + 2 {
            return Ok(DecompressOutcome {
                bytes_read: in_buf.len(),
                bytes_written: 0,
            });
        }

        let rest = if self.salt_value.is_none() {
            // the first few bytes contain the salt and password verification value
            let (salt_value, rest) = in_buf.split_at(salt_size);
            let (password_verification_value, rest) = rest.split_at(PASSWORD_VERIFICATION_SIZE);
            self.salt_value = Some(salt_value.to_vec());
            self.password_verification_value = Some(password_verification_value.to_vec());
            rest
        } else {
            in_buf
        };

        if matches!(has_more_input, HasMoreInput::No) {
            // the last few bytes contain the authentication code
            let (_rest, authentication_code) = rest.split_at(rest.len() - AUTHENTICATION_CODE_SIZE);
            self.authentication_code = Some(authentication_code.to_vec());
        }

        // copy the data to the output buffer to simulate decompression progress
        // we can't actually decrypt the data because we do not know the password
        let bytes_read = cmp::min(in_buf.len(), out_buf.len());
        out_buf[..bytes_read].copy_from_slice(&in_buf[..bytes_read]);

        Ok(DecompressOutcome {
            bytes_read,
            bytes_written: bytes_read,
        })
    }
}
