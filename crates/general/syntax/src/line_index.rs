#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineCol {
    pub line: u32,
    pub col: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Utf16LineCol {
    pub line: u32,
    pub col: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineIndex {
    line_starts: Vec<usize>,
    text: String,
}

impl LineIndex {
    pub fn new(text: &str) -> Self {
        let mut line_starts = vec![0];
        for (idx, ch) in text.char_indices() {
            if ch == '\n' {
                line_starts.push(idx + 1);
            }
        }
        Self {
            line_starts,
            text: text.to_string(),
        }
    }

    pub fn line_col(&self, offset: usize) -> LineCol {
        let offset = offset.min(self.text.len());
        let line = match self.line_starts.binary_search(&offset) {
            Ok(line) => line,
            Err(next) => next.saturating_sub(1),
        };
        LineCol {
            line: line as u32,
            col: offset.saturating_sub(self.line_starts[line]) as u32,
        }
    }

    pub fn utf16_line_col(&self, offset: usize) -> Utf16LineCol {
        let offset = offset.min(self.text.len());
        let line_col = self.line_col(offset);
        let line_start = self.line_starts[line_col.line as usize];
        let col = self.text[line_start..offset].encode_utf16().count();
        Utf16LineCol {
            line: line_col.line,
            col: col as u32,
        }
    }
}
