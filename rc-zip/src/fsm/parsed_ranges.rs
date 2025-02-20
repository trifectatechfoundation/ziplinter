use std::ops::Range;

#[derive(serde::Serialize, Debug, Clone)]
struct ParsedRange {
    /// Start of range
    start: u64,
    /// End of range (excluding)
    end: u64,
    /// The kind of data that was parsed here
    contains: &'static str,
    /// Additional info (e.g. filename for files)
    #[serde(skip_serializing_if = "Option::is_none")]
    filename: Option<String>,
}

#[derive(serde::Serialize, Debug, Clone)]
pub struct ParsedRanges(Vec<ParsedRange>);

impl ParsedRanges {
    pub fn new() -> Self {
        Self(Vec::new())
    }

    pub fn insert_range(
        &mut self,
        range: Range<u64>,
        contains: &'static str,
        filename: Option<String>,
    ) {
        self.0.push(ParsedRange {
            start: range.start,
            end: range.end,
            contains,
            filename,
        });
    }

    pub fn insert_offset_length(
        &mut self,
        offset: u64,
        length: u64,
        contains: &'static str,
        filename: Option<String>,
    ) {
        self.insert_range(offset..offset + length, contains, filename)
    }

    pub fn append(&mut self, other: &mut ParsedRanges) {
        self.0.append(&mut other.0);
    }
}
