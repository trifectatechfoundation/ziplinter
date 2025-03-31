use std::{fs::File, rc::Rc, sync::Mutex};

use rc_zip::{
    chrono::{DateTime, Utc},
    encoding::Encoding,
    fsm::{AexData, ParsedRanges},
    parse::{EndOfCentralDirectory, Entry, ExtraAexField, Method, MethodSpecific, Mode, Version},
};
use rc_zip_sync::{ArchiveHandle, EntryHandle, HasCursor, ReadZip};
use serde::ser::SerializeStruct;

#[derive(serde::Serialize)]
pub struct CentralDirectoryFileHeader {
    /// version made by
    pub creator_version: Version,

    /// version needed to extract
    pub reader_version: Version,

    /// general purpose bit flag
    pub flags: u16,

    /// compression method
    pub method: Method,

    /// last mod file datetime
    pub modified: DateTime<Utc>,

    /// crc32 hash
    pub crc32: u32,

    /// compressed size
    pub compressed_size: u32,

    /// uncompressed size
    pub uncompressed_size: u32,

    /// disk number start
    pub disk_nbr_start: u16,

    /// internal file attributes
    pub internal_attrs: u16,

    /// external file attributes
    pub external_attrs: u32,

    /// relative offset of local header
    pub header_offset: u32,

    /// name field
    pub name: String,

    /// extra field
    pub extra: Vec<u8>,

    /// comment field
    pub comment: String,

    /// File mode.
    pub mode: Mode,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub aex: Option<ExtraAexField>,
}

impl CentralDirectoryFileHeader {
    fn from_rc_zip(value: &rc_zip::parse::CentralDirectoryFileHeader<'_>, entry: &Entry) -> Self {
        CentralDirectoryFileHeader {
            creator_version: value.creator_version,
            reader_version: value.reader_version,
            flags: value.flags,
            method: value.method,
            modified: entry.modified,
            crc32: value.crc32,
            compressed_size: value.compressed_size,
            uncompressed_size: value.uncompressed_size,
            disk_nbr_start: value.disk_nbr_start,
            internal_attrs: value.internal_attrs,
            external_attrs: value.external_attrs,
            header_offset: value.header_offset,
            name: entry.name.clone(),
            extra: value.extra.to_vec(),
            comment: entry.comment.clone(),
            mode: entry.mode,
            aex: entry.aex,
        }
    }
}

#[derive(serde::Serialize)]
pub struct LocalFileHeader {
    /// version needed to extract
    pub reader_version: Version,

    /// general purpose bit flag
    pub flags: u16,

    /// compression method
    pub method: Method,

    /// last mod file datetime
    pub modified: DateTime<Utc>,

    /// This entry's "created" timestamp, if available.
    ///
    /// See [Self::modified] for caveats.
    pub created: Option<DateTime<Utc>>,

    /// This entry's "last accessed" timestamp, if available.
    ///
    /// See [Self::accessed] for caveats.
    pub accessed: Option<DateTime<Utc>>,

    /// crc-32
    pub crc32: u32,

    /// compressed size
    pub compressed_size: u64,

    /// uncompressed size
    pub uncompressed_size: u64,

    /// Offset of the local file header in the zip file
    ///
    /// ```text
    /// [optional non-zip data]
    /// [local file header 1] <------ header_offset points here
    /// [encryption header 1]
    /// [file data 1]
    /// [data descriptor 1]
    /// ...
    /// [central directory]
    /// [optional zip64 end of central directory info]
    /// [end of central directory record]
    /// ```
    pub header_offset: u64,

    /// Unix user ID
    ///
    /// Only present if a Unix extra field or New Unix extra field was found.
    pub uid: Option<u32>,

    /// Unix group ID
    ///
    /// Only present if a Unix extra field or New Unix extra field was found.
    pub gid: Option<u32>,

    /// file name
    pub name: String,

    /// extra field
    pub extra: Vec<u8>,

    /// method-specific fields
    pub method_specific: MethodSpecific,

    /// File mode.
    pub mode: Mode,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub aex: Option<ExtraAexField>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub aex_data: Option<AexData>,
}

impl LocalFileHeader {
    fn from_rc_zip<F: HasCursor>(
        entry: EntryHandle<'_, F>,
        parsed_ranges: Rc<Mutex<ParsedRanges>>,
    ) -> Result<Self, Error> {
        let (value, aex_data) = entry.local_header(parsed_ranges)?.ok_or(Error {
            error: format!("Can't get local file header for \"{}\"", entry.name),
        })?;
        let entry = value.as_entry()?;

        Ok(LocalFileHeader {
            reader_version: value.reader_version,
            flags: value.flags,
            method: value.method,
            modified: entry.modified,
            created: entry.created,
            accessed: entry.accessed,
            crc32: value.crc32,
            compressed_size: entry.compressed_size,
            uncompressed_size: entry.uncompressed_size,
            gid: entry.gid,
            uid: entry.uid,
            header_offset: entry.header_offset,
            name: entry.name,
            extra: value.extra.to_vec(),
            method_specific: value.method_specific,
            mode: entry.mode,
            aex: entry.aex,
            aex_data: aex_data.to_owned(),
        })
    }
}

/// File metadata which consists of an `Entry`, and some additional data from  the`CentralDirectoryFileHeader`
struct FileMetadata {
    central: CentralDirectoryFileHeader,
    local: Result<LocalFileHeader, Error>,
}

impl serde::Serialize for FileMetadata {
    // custom serialize implementation to unpack Result type
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut file_metadata = serializer.serialize_struct("FileMetadata", 2)?;
        file_metadata.serialize_field("central", &self.central)?;
        match &self.local {
            Ok(local) => file_metadata.serialize_field("local", &local)?,
            Err(error) => file_metadata.serialize_field("local", &error)?,
        }
        file_metadata.end()
    }
}

#[derive(serde::Serialize)]
struct ZipMetadata<'a> {
    eocd: &'a EndOfCentralDirectory<'static>,
    encoding: Encoding,
    size: u64,
    comment: &'a String,
    contents: Vec<FileMetadata>,
    parsed_ranges: ParsedRanges,
}

impl<'a, F> From<&'a mut ArchiveHandle<'a, F>> for ZipMetadata<'a>
where
    F: HasCursor,
{
    fn from(archive: &'a mut ArchiveHandle<'a, F>) -> Self {
        let contents = archive
            .entries()
            .zip(archive.directory_headers.iter())
            .map(|(entry, directory_header)| FileMetadata {
                central: CentralDirectoryFileHeader::from_rc_zip(directory_header, entry.entry),
                local: LocalFileHeader::from_rc_zip(entry, archive.parsed_ranges.clone()),
            })
            .collect();

        ZipMetadata {
            eocd: &archive.eocd,
            encoding: archive.encoding,
            size: archive.size,
            comment: &archive.comment,
            contents,
            parsed_ranges: archive.parsed_ranges.try_lock().unwrap().clone(),
        }
    }
}

#[derive(serde::Serialize)]
struct Error {
    error: String,
}

impl<T: std::fmt::Debug> From<T> for Error {
    fn from(error: T) -> Self {
        Error {
            error: format!("{:?}", error),
        }
    }
}

pub fn parse_bytes(bytes: &[u8]) -> serde_json::Value {
    match bytes.read_zip() {
        Ok(mut archive) => serde_json::to_value(ZipMetadata::from(&mut archive)).unwrap(),
        Err(error) => serde_json::to_value(Error::from(error)).unwrap(),
    }
}

pub fn parse_file(file: &File) -> serde_json::Value {
    match file.read_zip() {
        Ok(mut archive) => serde_json::to_value(ZipMetadata::from(&mut archive)).unwrap(),
        Err(error) => serde_json::to_value(Error::from(error)).unwrap(),
    }
}

#[cfg(test)]
mod test {
    use super::*;

    use insta::assert_json_snapshot;
    use std::{error::Error, path::Path};

    fn process_zip_file(zip_path: &Path) -> Result<serde_json::Value, Box<dyn Error>> {
        let file = std::fs::File::open(zip_path).unwrap();
        let mut archive = file.read_zip()?;

        let metadata = ZipMetadata::from(&mut archive);
        Ok(serde_json::to_value(metadata)?)
    }

    #[test]
    fn snapshot_zip_files() {
        let fixtures_dir = std::env::current_dir()
            .unwrap()
            .join("../testdata")
            .canonicalize()
            .unwrap();

        println!("fixtures_dir: {}", fixtures_dir.display());

        for entry in std::fs::read_dir(fixtures_dir).expect("Failed to read fixtures directory") {
            let entry = entry.expect("Failed to read entry");
            let path = entry.path();

            if path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("zip"))
            {
                let Ok(result) = process_zip_file(&path) else {
                    continue;
                };
                println!(
                    "current file: {}",
                    Path::new(path.file_name().unwrap()).display()
                );
                assert_json_snapshot!(
                    format!("{}", Path::new(path.file_name().unwrap()).display()),
                    result
                );
            }
        }
    }
}
