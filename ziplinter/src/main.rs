use std::ops::Deref;

fn main() {
    use rc_zip_sync::ReadZip;

    let mut args = std::env::args();
    let _ = args.next();
    let path = args.next().unwrap();

    let file = std::fs::File::open(path).unwrap();
    let archive = file.read_zip().unwrap();

    eprintln!("{}", serde_json::to_string(&archive.deref()).unwrap())
}
