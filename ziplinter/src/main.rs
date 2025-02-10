use rc_zip::{
    encoding::Encoding,
    parse::{Archive, CentralDirectoryFileHeader, EndOfCentralDirectory, Entry},
};
use rc_zip_sync::ReadZip;
use serde::Serialize;

/// File metadata which consists of an `Entry`, and some additional data from  the`CentralDirectoryFileHeader`
#[derive(Serialize)]
struct FileMetadata<'a> {
    #[serde(flatten)]
    entry: &'a Entry,
    #[serde(flatten)]
    directory_header: &'a CentralDirectoryFileHeader<'static>,
}

#[derive(serde::Serialize)]
struct ZipMetadata<'a> {
    eocd: &'a EndOfCentralDirectory<'static>,
    encoding: Encoding,
    size: u64,
    comment: &'a String,
    contents: Vec<FileMetadata<'a>>,
}

impl<'a> From<&'a Archive> for ZipMetadata<'a> {
    fn from(archive: &'a Archive) -> Self {
        ZipMetadata {
            eocd: &archive.eocd,
            encoding: archive.encoding,
            size: archive.size,
            comment: &archive.comment,
            contents: archive
                .entries
                .iter()
                .zip(archive.directory_headers.iter())
                .map(|(entry, directory_header)| FileMetadata {
                    entry,
                    directory_header,
                })
                .collect(),
        }
    }
}

fn main() {
    let mut args = std::env::args();
    let _ = args.next();
    let path = args.next().unwrap();

    let file = std::fs::File::open(path).unwrap();
    let archive = file.read_zip().unwrap();

    let metadata = ZipMetadata::from(&*archive);
    eprintln!("{}", serde_json::to_string_pretty(&metadata).unwrap())
}

#[cfg(test)]
mod test {
    use super::*;

    use insta::assert_json_snapshot;
    use std::{error::Error, ops::Deref, path::Path};

    fn process_zip_file(zip_path: &Path) -> Result<serde_json::Value, Box<dyn Error>> {
        let file = std::fs::File::open(zip_path).unwrap();
        let archive = file.read_zip();

        Ok(serde_json::to_value(archive?.deref())?)
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
