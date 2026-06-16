use crate::error::{ParseError, ParseErrorKind};
use crate::span::Span;

const MAX_DIAGNOSTICS: usize = 50;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Delim {
    Brace,
    Bracket,
    Paren,
}

#[derive(Debug, Clone, Copy)]
struct DelimFrame {
    delim: Delim,
    offset: usize,
    type_context: bool,
    saw_local_binding: bool,
}

#[derive(Debug, Clone, Copy, Default)]
struct Segment {
    first_compare: Option<usize>,
    pipeline_dir: Option<PipelineDir>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PipelineDir {
    Forward,
    Backward,
}

pub fn collect_common_diagnostics(src: &str) -> Vec<ParseError> {
    let mut scanner = Scanner::new(src);
    scanner.scan();
    scanner.errors.sort_by_key(|e| (e.span.start, e.span.end));
    scanner
        .errors
        .dedup_by(|a, b| a.span == b.span && a.kind == b.kind);
    scanner.errors.truncate(MAX_DIAGNOSTICS);
    scanner.errors
}

struct Scanner<'a> {
    src: &'a str,
    bytes: &'a [u8],
    pos: usize,
    stack: Vec<DelimFrame>,
    segments: Vec<Segment>,
    errors: Vec<ParseError>,
}

impl<'a> Scanner<'a> {
    fn new(src: &'a str) -> Self {
        Self {
            src,
            bytes: src.as_bytes(),
            pos: 0,
            stack: Vec::new(),
            segments: vec![Segment::default()],
            errors: Vec::new(),
        }
    }

    fn scan(&mut self) {
        while self.pos < self.bytes.len() && self.errors.len() < MAX_DIAGNOSTICS {
            if self.starts_with("--[") {
                self.skip_block_comment();
                continue;
            }
            if self.starts_with("--|") || self.starts_with("--") {
                self.skip_line_comment();
                continue;
            }
            if self.bytes[self.pos] == b'"' {
                self.skip_string();
                continue;
            }

            match self.bytes[self.pos] {
                b'{' => self.open(Delim::Brace),
                b'[' => self.open(Delim::Bracket),
                b'(' => self.open(Delim::Paren),
                b'}' => self.close(Delim::Brace),
                b']' => self.close(Delim::Bracket),
                b')' => self.close(Delim::Paren),
                b';' | b',' => {
                    self.reset_segment();
                    self.pos += 1;
                }
                b'\n' if self.stack.is_empty() => {
                    self.reset_segment();
                    self.pos += 1;
                }
                b':' => self.scan_colon(),
                b'=' => self.scan_equals(),
                b'\\' => self.scan_lambda(),
                b'<' | b'>' | b'!' => self.scan_operator(),
                b'|' => self.scan_pipeline(),
                _ => self.pos += self.char_len_at(self.pos),
            }
        }

        while let Some(frame) = self.stack.pop() {
            self.push(
                frame.offset,
                frame.offset + 1,
                ParseErrorKind::UnclosedDelimiter(match frame.delim {
                    Delim::Brace => '{',
                    Delim::Bracket => '[',
                    Delim::Paren => '(',
                }),
            );
        }
    }

    fn open(&mut self, delim: Delim) {
        let type_context = self.is_type_context_before(self.pos);
        self.stack.push(DelimFrame {
            delim,
            offset: self.pos,
            type_context,
            saw_local_binding: false,
        });
        self.segments.push(Segment::default());
        self.pos += 1;
    }

    fn close(&mut self, delim: Delim) {
        match delim {
            Delim::Bracket => self.check_missing_list_semicolon(),
            Delim::Brace => self.check_missing_block_result(),
            Delim::Paren => {}
        }
        if let Some(frame) = self.stack.pop()
            && frame.delim != delim
        {
            self.push(
                self.pos,
                self.pos + 1,
                ParseErrorKind::ExpectedToken("matching delimiter"),
            );
        }
        self.segments.pop();
        if self.segments.is_empty() {
            self.segments.push(Segment::default());
        }
        self.pos += 1;
    }

    fn scan_colon(&mut self) {
        if self.starts_with("::") || self.starts_with(":=") {
            if self.starts_with(":=") {
                self.mark_local_binding();
            }
            self.pos += 2;
            return;
        }

        if self.stack.is_empty() && self.looks_like_top_level_single_colon(self.pos) {
            self.push(self.pos, self.pos + 1, ParseErrorKind::TopLevelSingleColon);
        } else if self.in_value_brace() && self.looks_like_value_record_colon(self.pos) {
            self.push(
                self.pos,
                self.pos + 1,
                ParseErrorKind::ValueRecordFieldUsesColon,
            );
        }
        self.pos += 1;
    }

    fn scan_equals(&mut self) {
        if self.starts_with("=>") {
            self.pos += 2;
            return;
        }
        if self.in_type_brace() && self.looks_like_type_record_equals(self.pos) {
            self.push(
                self.pos,
                self.pos + 1,
                ParseErrorKind::TypeRecordFieldUsesEquals,
            );
        }
        self.pos += 1;
    }

    fn scan_lambda(&mut self) {
        let lambda_start = self.pos;
        let end = self.find_expr_boundary(lambda_start + 1);
        if let Some(rel) = self.src[lambda_start..end].find("=>") {
            let offset = lambda_start + rel;
            self.push(offset, offset + 2, ParseErrorKind::LambdaArrow);
        }
        if let Some(dot) = self.find_lambda_dot(lambda_start + 1, end) {
            let after = dot + 1;
            if after >= self.bytes.len() || !self.bytes[after].is_ascii_whitespace() {
                self.push(dot, dot + 1, ParseErrorKind::LambdaDotNeedsWhitespace);
            }
        }
        self.pos += 1;
    }

    fn scan_operator(&mut self) {
        if self.in_type_position_near(self.pos) {
            self.pos += 1;
            return;
        }
        if let Some((len, is_compare)) = self.operator_at(self.pos) {
            if is_compare {
                let offset = self.pos;
                let segment = self.current_segment_mut();
                if segment.first_compare.is_some() {
                    self.push(offset, offset + len, ParseErrorKind::ChainedComparison);
                } else {
                    segment.first_compare = Some(offset);
                }
            }
            self.pos += len;
        } else {
            self.pos += 1;
        }
    }

    fn scan_pipeline(&mut self) {
        if self.starts_with("|>") {
            self.note_pipeline(PipelineDir::Forward, self.pos);
            self.pos += 2;
        } else {
            self.pos += 1;
        }
    }

    fn note_pipeline(&mut self, dir: PipelineDir, offset: usize) {
        let segment = self.current_segment_mut();
        match segment.pipeline_dir {
            Some(prev) if prev != dir => {
                self.push(offset, offset + 2, ParseErrorKind::MixedPipeline);
            }
            None => segment.pipeline_dir = Some(dir),
            _ => {}
        }
    }

    fn operator_at(&mut self, offset: usize) -> Option<(usize, bool)> {
        if self.starts_at(offset, "<|") {
            self.note_pipeline(PipelineDir::Backward, offset);
            return Some((2, false));
        }
        if offset > 0 && matches!(self.bytes[offset - 1], b'-' | b'=') {
            return None;
        }
        for op in ["==", "!=", "<=", ">="] {
            if self.starts_at(offset, op) {
                return Some((2, true));
            }
        }
        if self.starts_at(offset, "<") && !self.looks_like_type_param_angle(offset) {
            return Some((1, true));
        }
        if self.starts_at(offset, ">") && !self.looks_like_type_param_close(offset) {
            return Some((1, true));
        }
        None
    }

    fn check_missing_list_semicolon(&mut self) {
        if !matches!(self.stack.last().map(|f| f.delim), Some(Delim::Bracket)) {
            return;
        }
        let open = self.stack.last().map(|f| f.offset).unwrap_or(0);
        let Some(prev) = self.prev_non_ws(self.pos) else {
            return;
        };
        if prev <= open || self.bytes[prev] == b'[' || self.bytes[prev] == b';' {
            return;
        }
        self.push(
            self.pos,
            self.pos + 1,
            ParseErrorKind::MissingListItemSemicolon,
        );
    }

    fn check_missing_block_result(&mut self) {
        if !self.in_value_brace() {
            return;
        }
        let frame = self.stack.last().copied();
        let open = frame.map(|f| f.offset).unwrap_or(0);
        let Some(prev) = self.prev_non_ws(self.pos) else {
            return;
        };
        if prev <= open || self.bytes[prev] != b';' {
            return;
        }
        if frame.is_some_and(|f| f.saw_local_binding) {
            self.push(self.pos, self.pos + 1, ParseErrorKind::MissingBlockResult);
        }
    }

    fn reset_segment(&mut self) {
        *self.current_segment_mut() = Segment::default();
    }

    fn push(&mut self, start: usize, end: usize, kind: ParseErrorKind) {
        if self.errors.len() >= MAX_DIAGNOSTICS {
            return;
        }
        let clamped_start = start.min(self.bytes.len());
        let clamped_end = end.min(self.bytes.len()).max(clamped_start);
        self.errors.push(ParseError::from_kind(
            Span::new(clamped_start, clamped_end),
            kind,
        ));
    }

    fn current_segment_mut(&mut self) -> &mut Segment {
        self.segments
            .last_mut()
            .expect("scanner always has a segment")
    }

    fn starts_with(&self, s: &str) -> bool {
        self.starts_at(self.pos, s)
    }

    fn starts_at(&self, offset: usize, s: &str) -> bool {
        self.src[offset..].starts_with(s)
    }

    fn char_len_at(&self, offset: usize) -> usize {
        self.src[offset..].chars().next().map_or(1, char::len_utf8)
    }

    fn skip_line_comment(&mut self) {
        while self.pos < self.bytes.len() && self.bytes[self.pos] != b'\n' {
            self.pos += 1;
        }
    }

    fn skip_block_comment(&mut self) {
        self.pos += 3;
        let mut depth = 1usize;
        while self.pos < self.bytes.len() && depth > 0 {
            if self.starts_with("--[") {
                depth += 1;
                self.pos += 3;
            } else if self.starts_with("]--") {
                depth -= 1;
                self.pos += 3;
            } else {
                self.pos += self.char_len_at(self.pos);
            }
        }
    }

    fn skip_string(&mut self) {
        self.pos += 1;
        while self.pos < self.bytes.len() {
            match self.bytes[self.pos] {
                b'\\' => {
                    self.pos += 1;
                    if self.pos < self.bytes.len() {
                        self.pos += self.char_len_at(self.pos);
                    }
                }
                b'"' => {
                    self.pos += 1;
                    break;
                }
                _ => self.pos += self.char_len_at(self.pos),
            }
        }
    }

    fn prev_non_ws(&self, offset: usize) -> Option<usize> {
        self.src[..offset]
            .char_indices()
            .rev()
            .find(|(_, ch)| !ch.is_whitespace())
            .map(|(idx, _)| idx)
    }

    fn next_non_ws(&self, offset: usize) -> Option<(usize, char)> {
        self.src[offset..]
            .char_indices()
            .find(|(_, ch)| !ch.is_whitespace())
            .map(|(idx, ch)| (offset + idx, ch))
    }

    fn is_type_context_before(&self, offset: usize) -> bool {
        let before = self.src[..offset].trim_end();
        before.ends_with("type")
            // Single `:` (e.g. `field : {`) but NOT `::` (e.g. `Eq @Int :: {` is a witness body)
            || (before.ends_with(':') && !before.ends_with("::"))
            || before.ends_with("->")
            || before.ends_with('[')
            || self.stack.last().is_some_and(|f| f.type_context)
    }

    fn in_value_brace(&self) -> bool {
        matches!(
            self.stack.last(),
            Some(DelimFrame {
                delim: Delim::Brace,
                type_context: false,
                ..
            })
        )
    }

    fn in_type_brace(&self) -> bool {
        matches!(
            self.stack.last(),
            Some(DelimFrame {
                delim: Delim::Brace,
                type_context: true,
                ..
            })
        )
    }

    fn mark_local_binding(&mut self) {
        if let Some(frame) = self.stack.last_mut()
            && frame.delim == Delim::Brace
            && !frame.type_context
        {
            frame.saw_local_binding = true;
        }
    }

    fn looks_like_top_level_single_colon(&self, offset: usize) -> bool {
        let line_start = self.src[..offset].rfind('\n').map_or(0, |idx| idx + 1);
        let prefix = self.src[line_start..offset].trim();
        if prefix.is_empty() {
            return false;
        }
        if !prefix
            .chars()
            .next()
            .is_some_and(|ch| ch.is_ascii_alphabetic() || ch == '_')
        {
            return false;
        }
        // If we're inside an unmatched `<...>` type-param list (line has `::` and a `<`
        // that hasn't been closed yet), this `:` is a bound separator, not a stray colon.
        let line_text = &self.src[line_start..offset];
        if line_text.contains("::") && line_text.contains('<') {
            let depth: i32 = line_text.chars().fold(0i32, |d, c| {
                if c == '<' {
                    d + 1
                } else if c == '>' {
                    d - 1
                } else {
                    d
                }
            });
            if depth > 0 {
                return false;
            }
        }
        true
    }

    fn looks_like_value_record_colon(&self, offset: usize) -> bool {
        let Some((_, next)) = self.next_non_ws(offset + 1) else {
            return false;
        };
        matches!(next, '0'..='9' | '-' | '"' | '#' | '[' | '{' | '(')
            || self.src[offset + 1..].trim_start().starts_with("true")
            || self.src[offset + 1..].trim_start().starts_with("false")
    }

    fn looks_like_type_record_equals(&self, offset: usize) -> bool {
        let line_start = self.src[..offset]
            .rfind(['\n', ';', '{'])
            .map_or(0, |idx| idx + 1);
        let prefix = self.src[line_start..offset].trim();
        !prefix.is_empty()
            && prefix
                .chars()
                .next()
                .is_some_and(|ch| ch.is_ascii_alphabetic() || ch == '_')
    }

    fn looks_like_type_param_angle(&self, offset: usize) -> bool {
        let before = self.src[..offset].trim_end();
        before.ends_with("::")
    }

    fn looks_like_type_param_close(&self, offset: usize) -> bool {
        let line_start = self.src[..offset].rfind('\n').map_or(0, |idx| idx + 1);
        let before = &self.src[line_start..offset];
        before.contains("::") && before.contains('<')
    }

    fn in_type_position_near(&self, offset: usize) -> bool {
        if self.stack.last().is_some_and(|f| f.type_context) {
            return true;
        }
        let line_start = self.src[..offset]
            .rfind(['\n', ';', '{', '}'])
            .map_or(0, |idx| idx + 1);
        let before = &self.src[line_start..offset];
        before.contains("::") || before.contains("= type ")
    }

    fn find_expr_boundary(&self, start: usize) -> usize {
        let mut offset = start;
        while offset < self.bytes.len() {
            match self.bytes[offset] {
                b';' | b'}' | b']' | b')' | b'\n' => return offset,
                b'"' => {
                    offset += 1;
                    while offset < self.bytes.len() && self.bytes[offset] != b'"' {
                        if self.bytes[offset] == b'\\' {
                            offset += 1;
                        }
                        offset += self.char_len_at(offset);
                    }
                }
                _ => offset += self.char_len_at(offset),
            }
        }
        offset
    }

    fn find_lambda_dot(&self, start: usize, end: usize) -> Option<usize> {
        let mut offset = start;
        while offset < end {
            if self.bytes[offset] == b'.' {
                return Some(offset);
            }
            offset += self.char_len_at(offset);
        }
        None
    }
}
