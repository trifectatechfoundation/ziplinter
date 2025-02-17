use std::ops::Range;

#[derive(serde::Serialize)]
struct ParsedRange {
    /// Start of range
    start: u64,
    /// End of range (excluding)
    end: u64,
    /// The kind of data that was parsed here
    kind: &'static str,
    /// Additional info (e.g. filename for files)
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
}

#[derive(serde::Serialize)]
pub struct ParsedRanges(Vec<ParsedRange>);

impl ParsedRanges {
    pub fn new() -> Self {
        Self(Vec::new())
    }

    pub fn insert_range(
        &mut self,
        range: Range<u64>,
        kind: &'static str,
        description: Option<String>,
    ) {
        self.0.push(ParsedRange {
            start: range.start,
            end: range.end,
            kind,
            description,
        });
    }

    pub fn insert_offset_length(
        &mut self,
        offset: u64,
        length: u64,
        kind: &'static str,
        description: Option<String>,
    ) {
        self.insert_range(offset..offset + length, kind, description)
    }
}
