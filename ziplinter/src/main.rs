fn main() {
    #[cfg(feature = "tracing")]
    {
        let subscriber = tracing_subscriber::FmtSubscriber::builder()
            .with_max_level(tracing::Level::TRACE)
            .finish();
        tracing::subscriber::set_global_default(subscriber).unwrap();
    }

    let mut args = std::env::args();
    let _ = args.next();
    let path = args
        .next()
        .expect("Please provide a path to the zip file to analyze");

    let file = std::fs::File::open(path).unwrap();
    let value = ziplinter::parse_file(&file);
    println!("{}", serde_json::to_string_pretty(&value).unwrap());
}
