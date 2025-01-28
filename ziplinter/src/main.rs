use rc_zip_sync::ReadZip;
use std::ops::Deref;

fn main() {
    let mut args = std::env::args();
    let _ = args.next();
    let path = args.next().unwrap();

    let file = std::fs::File::open(path).unwrap();
    let archive = file.read_zip().unwrap();

    eprintln!(
        "{}",
        serde_json::to_string_pretty(&archive.deref()).unwrap()
    )
}

#[cfg(test)]
mod test {
    use super::*;

    use insta::assert_json_snapshot;
    use std::{error::Error, path::Path};

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
