[![MIT OR Apache-2.0 licensed](https://img.shields.io/badge/license-MIT+Apache_2.0-blue.svg)](./LICENSE)

# ziplinter 

A zip file analyzer

## Installation
Install Rust as described [here](https://www.rust-lang.org/tools/install), then build the project using:

```
cargo build --release
```

Optionally, you can enable traces when building to print additional debug information:
```
cargo build --release --features tracing
```

## Usage
Once the project is built, you can run the ziplinter by giving it a path to a zip file:
```
./target/release/ziplinter ./testdata/test.zip
```
Ziplinter will then read the zip to gather metadata, which is then printed to standard output in JSON format. The JSON format contains the following properties:
- `comment`: the top level archive comment
- `contents`: the metadata for the files in zip, both from the central directory and from the local file headers
- `encoding`: the text encoding used, e.g. `Utf8`
- `eocd`: the end of central directory information, which is used to locate the central directory
- `size`: the size of the zip file in bytes
- `parsed_ranges`: a list of the ranges within the zip file that were parsed

See the [snapshots folder](https://github.com/trifectatechfoundation/ziplinter/tree/main/ziplinter/src/snapshots) for examples of the JSON output for the various test zips from the `testdata` directory.

## Thanks

The internals heavily rely on [rc-zip](https://github.com/bearcove/rc-zip). We'd use it as a dependency, but need access to some internals that don't make much sense for a general audience. So, this is basically a fork that exposes more things.

## License

This project is primarily distributed under the terms of both the MIT license
and the Apache License (Version 2.0).

See [LICENSE-APACHE](LICENSE-APACHE) and [LICENSE-MIT](LICENSE-MIT) for details.
