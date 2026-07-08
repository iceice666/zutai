use crate::error::{ParseError, ParseErrorKind};
use crate::numlit::{PostfixCheck, classify_postfix};
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
            if self.is_number_start() {
                self.scan_number();
                continue;
            }

            if self.starts_keyword_at(self.pos, "type")
                && self.looks_like_stale_type_declaration(self.pos)
            {
                self.push(
                    self.pos,
                    self.pos + "type".len(),
                    ParseErrorKind::StaleTypeDeclaration,
                );
                self.pos += "type".len();
                continue;
            }

            if self
                .src
                .get(self.pos..)
                .and_then(|rest| rest.chars().next())
                .is_some_and(crate::ident::is_ident_start)
            {
                self.scan_identifier_like();
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
                b'+' | b'*' | b'/' => {
                    self.check_trailing_operator(self.pos, 1);
                    self.pos += 1;
                }
                b'-' => {
                    self.check_trailing_operator(self.pos, 1);
                    self.pos += 1;
                }
                b'.' => self.scan_access_operator(1),
                b'&' => {
                    if self.starts_with("&&") {
                        self.break_compare_chain();
                        self.check_trailing_operator(self.pos, 2);
                        self.pos += 2;
                    } else {
                        self.pos += 1;
                    }
                }
                b'?' => {
                    if self.starts_with("?.") {
                        self.scan_access_operator(2);
                    } else if self.starts_with("??") {
                        self.break_compare_chain();
                        self.check_trailing_operator(self.pos, 2);
                        self.pos += 2;
                    } else {
                        self.pos += 1;
                    }
                }
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
        });
        self.segments.push(Segment::default());
        self.pos += 1;
    }

    fn close(&mut self, delim: Delim) {
        match delim {
            // `{ ... }` value braces (record or list) require every item to be
            // `;`-terminated. `[ ... ]` do-blocks do not (the tail result has no
            // trailing `;`), so they get no item-semicolon check.
            Delim::Brace => self.check_missing_value_item_semicolon(),
            Delim::Bracket | Delim::Paren => {}
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
        // `::` (top-level sig/alias) and `:=` (local do-block binding) are not
        // stray colons.
        if self.starts_with("::") {
            if self.in_do_block() && self.looks_like_local_binding_double_colon(self.pos) {
                self.push(
                    self.pos,
                    self.pos + 2,
                    ParseErrorKind::LocalBindingDoubleColon,
                );
            }
            self.pos += 2;
            return;
        }
        if self.starts_with(":=") {
            self.pos += 2;
            return;
        }

        if !self.in_type_brace() && self.previous_atom_before(self.pos) {
            self.push(
                self.pos,
                self.pos + 1,
                ParseErrorKind::TaggedValuePayloadUsesColon,
            );
        } else if self.stack.is_empty() && self.looks_like_top_level_single_colon(self.pos) {
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
        if self.in_type_brace() && self.previous_atom_before(self.pos) {
            self.push(
                self.pos,
                self.pos + 1,
                ParseErrorKind::TypeUnionPayloadUsesEquals,
            );
        } else if self.in_type_brace() && self.looks_like_type_record_equals(self.pos) {
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
            if after >= self.bytes.len()
                || !self.src[after..]
                    .chars()
                    .next()
                    .is_some_and(char::is_whitespace)
            {
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
            self.check_trailing_operator(self.pos, len);
            self.pos += len;
        } else {
            self.pos += 1;
        }
    }

    fn scan_pipeline(&mut self) {
        if self.starts_with("|>") {
            self.note_pipeline(PipelineDir::Forward, self.pos);
            self.break_compare_chain();
            self.check_trailing_operator(self.pos, 2);
            self.pos += 2;
        } else if self.starts_with("||") {
            self.break_compare_chain();
            self.check_trailing_operator(self.pos, 2);
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

    fn scan_number(&mut self) {
        let start = self.pos;
        let has_sign = self.bytes[self.pos] == b'-';
        if has_sign {
            self.pos += 1;
        }

        self.consume_ascii_digits();

        let mut has_frac_or_exp = false;
        if self.pos + 1 < self.bytes.len()
            && self.bytes[self.pos] == b'.'
            && self.bytes[self.pos + 1].is_ascii_digit()
        {
            has_frac_or_exp = true;
            self.pos += 1;
            self.consume_ascii_digits();
        }

        if self.pos < self.bytes.len() && matches!(self.bytes[self.pos], b'e' | b'E') {
            let exp = self.pos;
            self.pos += 1;
            if self.pos < self.bytes.len() && matches!(self.bytes[self.pos], b'+' | b'-') {
                self.pos += 1;
            }
            let digit_start = self.pos;
            self.consume_ascii_digits();
            if self.pos > digit_start {
                has_frac_or_exp = true;
            } else {
                self.pos = exp;
            }
        }

        let body_end = self.pos;
        self.consume_postfix_run();
        let run = &self.src[body_end..self.pos];
        let kind = match classify_postfix(run, has_sign, has_frac_or_exp) {
            PostfixCheck::Unknown => Some(ParseErrorKind::UnknownNumberPostfix),
            PostfixCheck::IntOnFloatBody => Some(ParseErrorKind::IntegerPostfixOnFloatLiteral),
            PostfixCheck::UnsignedNegative => Some(ParseErrorKind::UnsignedPostfixOnNegative),
            PostfixCheck::None | PostfixCheck::Valid(_) => None,
        };
        if let Some(kind) = kind {
            self.push(start, self.pos, kind);
        }
    }

    fn is_number_start(&self) -> bool {
        self.at_number_boundary()
            && (self.bytes[self.pos].is_ascii_digit()
                || (self.bytes[self.pos] == b'-'
                    && self.pos + 1 < self.bytes.len()
                    && self.bytes[self.pos + 1].is_ascii_digit()))
    }

    fn at_number_boundary(&self) -> bool {
        self.previous_char_before(self.pos).is_none_or(|ch| {
            !(ch.is_ascii_alphanumeric() || ch == '_' || crate::ident::is_ident_continue(ch))
        })
    }

    fn consume_ascii_digits(&mut self) {
        while self.pos < self.bytes.len() && self.bytes[self.pos].is_ascii_digit() {
            self.pos += 1;
        }
    }

    fn scan_identifier_like(&mut self) {
        self.pos += self.char_len_at(self.pos);
        while self.pos < self.bytes.len() {
            let Some(ch) = self.src[self.pos..].chars().next() else {
                break;
            };
            if crate::ident::is_ident_continue(ch) || ch == '\'' {
                self.pos += ch.len_utf8();
            } else {
                break;
            }
        }
        if self.starts_with("?") && !self.starts_with("?.") && !self.starts_with("??") {
            self.pos += 1;
        }
    }

    fn consume_postfix_run(&mut self) {
        while self.pos < self.bytes.len()
            && (self.bytes[self.pos].is_ascii_alphanumeric() || self.bytes[self.pos] == b'_')
        {
            self.pos += 1;
        }
    }

    fn operator_at(&mut self, offset: usize) -> Option<(usize, bool)> {
        if self.starts_at(offset, ">>=") {
            return Some((3, false));
        }
        if self.starts_at(offset, "<|") {
            self.note_pipeline(PipelineDir::Backward, offset);
            self.break_compare_chain();
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

    fn scan_access_operator(&mut self, len: usize) {
        if len == 1
            && (self.starts_with("..")
                || self.pos > 0 && self.bytes[self.pos - 1] == b'.'
                || self.looks_like_lambda_dot(self.pos))
        {
            self.pos += 1;
            return;
        }
        let after = self.skip_trivia_from(self.pos + len);
        if after >= self.bytes.len()
            || !self.src[after..]
                .chars()
                .next()
                .is_some_and(crate::ident::is_ident_start)
        {
            self.push(
                self.pos,
                self.pos + len,
                ParseErrorKind::MissingFieldAfterAccess,
            );
        }
        self.pos += len;
    }

    fn check_trailing_operator(&mut self, start: usize, len: usize) {
        if self.looks_like_parenthesized_operator_name(start, len) {
            return;
        }
        let after = self.skip_trivia_from(start + len);
        if after >= self.bytes.len()
            || matches!(self.bytes[after], b';' | b'}' | b']' | b')' | b',')
        {
            self.push(start, start + len, ParseErrorKind::TrailingOperator);
        }
    }

    /// Every item in a value `{ ... }` (record field or list element) must end
    /// in `;`. If the last non-whitespace byte before `}` is neither the opening
    /// `{` (empty record) nor a `;`, an item terminator is missing.
    fn check_missing_value_item_semicolon(&mut self) {
        if !self.in_value_brace() {
            return;
        }
        let open = self.stack.last().map(|f| f.offset).unwrap_or(0);
        let Some(prev) = self.prev_non_ws(self.pos) else {
            return;
        };
        // List cons patterns end with the tail binding (`{h; ...t}`), not a
        // value-item semicolon. They are pattern syntax, not value braces.
        if self.src[open..self.pos].contains("...") {
            return;
        }
        // Fine: empty brace (`{`), a properly terminated item (`;`), or a last
        // item that ends in a closing delimiter — `}`/`]`/`)`. The closer case
        // covers brace groups that are not bare record/list items (generator
        // `stream { … then { … } }`, a trailing do-block, a nested list/tuple);
        // any genuine missing `;` there is reported by the real parser instead.
        if prev <= open || matches!(self.bytes[prev], b'{' | b';' | b'}' | b']' | b')') {
            return;
        }
        self.push(
            self.pos,
            self.pos + 1,
            ParseErrorKind::MissingListItemSemicolon,
        );
    }

    fn reset_segment(&mut self) {
        *self.current_segment_mut() = Segment::default();
    }

    /// Operators that bind looser than comparison (`&&`, `||`, `??`, `|>`, `<|`)
    /// separate independent comparison operands, so a comparison after one of
    /// them does not chain with a comparison before it. Reset the chained-
    /// comparison tracker without disturbing pipeline-direction tracking.
    fn break_compare_chain(&mut self) {
        self.current_segment_mut().first_compare = None;
    }

    fn push(&mut self, start: usize, end: usize, kind: ParseErrorKind) {
        if self.errors.len() >= MAX_DIAGNOSTICS {
            return;
        }
        let clamped_start = self.floor_char_boundary(start.min(self.bytes.len()));
        let clamped_end = self
            .ceil_char_boundary(end.min(self.bytes.len()))
            .max(clamped_start);
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

    fn previous_char_before(&self, offset: usize) -> Option<char> {
        self.src[..offset].chars().next_back()
    }

    fn floor_char_boundary(&self, mut offset: usize) -> usize {
        offset = offset.min(self.bytes.len());
        while offset > 0 && !self.src.is_char_boundary(offset) {
            offset -= 1;
        }
        offset
    }

    fn ceil_char_boundary(&self, mut offset: usize) -> usize {
        offset = offset.min(self.bytes.len());
        while offset < self.bytes.len() && !self.src.is_char_boundary(offset) {
            offset += 1;
        }
        offset
    }

    fn starts_keyword_at(&self, offset: usize, keyword: &str) -> bool {
        if !self.starts_at(offset, keyword) {
            return false;
        }
        let after = offset + keyword.len();
        after >= self.bytes.len()
            || !self.src[after..]
                .chars()
                .next()
                .is_some_and(crate::ident::is_ident_continue)
    }

    fn skip_trivia_from(&self, mut offset: usize) -> usize {
        while offset < self.bytes.len() {
            if self.src[offset..]
                .chars()
                .next()
                .is_some_and(char::is_whitespace)
            {
                offset += self.char_len_at(offset);
            } else if self.starts_at(offset, "--[") {
                offset += 3;
                let mut depth = 1usize;
                while offset < self.bytes.len() && depth > 0 {
                    if self.starts_at(offset, "--[") {
                        depth += 1;
                        offset += 3;
                    } else if self.starts_at(offset, "]--") {
                        depth -= 1;
                        offset += 3;
                    } else {
                        offset += self.char_len_at(offset);
                    }
                }
                if depth > 0 {
                    return self.bytes.len();
                }
            } else if self.starts_at(offset, "--|") || self.starts_at(offset, "--") {
                while offset < self.bytes.len() && self.bytes[offset] != b'\n' {
                    offset += self.char_len_at(offset);
                }
            } else {
                break;
            }
        }
        offset
    }

    fn previous_atom_before(&self, offset: usize) -> bool {
        let Some(prev) = self.prev_non_ws(offset) else {
            return false;
        };
        if !self.src.is_char_boundary(prev) {
            return false;
        }
        let Some(prev_ch) = self.src[prev..].chars().next() else {
            return false;
        };
        if !crate::ident::is_atom_continue(prev_ch) {
            return false;
        }
        let mut start = prev;
        while let Some((idx, ch)) = self.src[..start].char_indices().next_back() {
            if crate::ident::is_atom_continue(ch) {
                start = idx;
            } else {
                break;
            }
        }
        self.src[..start]
            .char_indices()
            .next_back()
            .is_some_and(|(_, ch)| ch == '#')
    }

    fn looks_like_lambda_dot(&self, offset: usize) -> bool {
        let segment_start = self.src[..offset]
            .rfind(['\n', ';', '{', '[', '(', '}', ']', ')'])
            .map_or(0, |idx| idx + 1);
        let Some(backslash) = self.src[segment_start..offset].rfind('\\') else {
            return false;
        };
        let lambda_head = &self.src[segment_start + backslash + 1..offset];
        !lambda_head.is_empty()
            && !lambda_head.contains('.')
            && lambda_head.chars().all(|ch| {
                ch.is_whitespace()
                    || crate::ident::is_ident_continue(ch)
                    || matches!(ch, '_' | '(' | ')' | ',' | '{' | '}' | ';' | '#')
            })
    }

    fn looks_like_parenthesized_operator_name(&self, start: usize, len: usize) -> bool {
        self.prev_non_ws(start)
            .is_some_and(|prev| self.bytes[prev] == b'(')
            && self.skip_trivia_from(start + len) < self.bytes.len()
            && self.bytes[self.skip_trivia_from(start + len)] == b')'
    }

    fn in_do_block(&self) -> bool {
        matches!(
            self.stack.last(),
            Some(DelimFrame {
                delim: Delim::Bracket,
                ..
            })
        )
    }

    fn skip_line_comment(&mut self) {
        while self.pos < self.bytes.len() && self.bytes[self.pos] != b'\n' {
            self.pos += self.char_len_at(self.pos);
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
            || self.looks_like_type_after_params(offset)
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

    fn looks_like_top_level_single_colon(&self, offset: usize) -> bool {
        let line_start = self.src[..offset].rfind('\n').map_or(0, |idx| idx + 1);
        let prefix = self.src[line_start..offset].trim();
        if prefix.is_empty() {
            return false;
        }
        if !prefix
            .chars()
            .next()
            .is_some_and(crate::ident::is_ident_start)
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
                .is_some_and(crate::ident::is_ident_start)
    }

    fn looks_like_stale_type_declaration(&self, offset: usize) -> bool {
        if !self.stack.is_empty() {
            return false;
        }
        let line_start = self.src[..offset].rfind('\n').map_or(0, |idx| idx + 1);
        if !self.src[line_start..offset].trim().is_empty() {
            return false;
        }
        let mut cursor = self.skip_trivia_from(offset + "type".len());
        if cursor >= self.bytes.len() {
            return false;
        }
        let Some(first) = self.src[cursor..].chars().next() else {
            return false;
        };
        if !crate::ident::is_ident_start(first) {
            return false;
        }
        cursor += first.len_utf8();
        while cursor < self.bytes.len() {
            let Some(ch) = self.src[cursor..].chars().next() else {
                break;
            };
            if crate::ident::is_ident_continue(ch) {
                cursor += ch.len_utf8();
            } else {
                break;
            }
        }
        cursor = self.skip_trivia_from(cursor);
        cursor < self.bytes.len() && self.bytes[cursor] == b'='
    }

    fn looks_like_local_binding_double_colon(&self, offset: usize) -> bool {
        let start = self.src[..offset]
            .rfind(['[', ';'])
            .map_or(0, |idx| idx + 1);
        let prefix = self.src[start..offset].trim();
        if prefix.is_empty() {
            return false;
        }
        let mut chars = prefix.chars();
        if !chars.next().is_some_and(crate::ident::is_ident_start)
            || !chars.all(crate::ident::is_ident_continue)
        {
            return false;
        }
        let mut cursor = offset + 2;
        while cursor < self.bytes.len() && !matches!(self.bytes[cursor], b';' | b']') {
            if self.bytes[cursor] == b'=' {
                return true;
            }
            cursor += self.char_len_at(cursor);
        }
        false
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

    fn looks_like_type_after_params(&self, offset: usize) -> bool {
        let before = self.src[..offset].trim_end();
        if !before.ends_with('>') {
            return false;
        }
        let line_start = self.src[..offset].rfind('\n').map_or(0, |idx| idx + 1);
        let line = &self.src[line_start..offset];
        let Some(sig) = line.rfind("::") else {
            return false;
        };
        let Some(params) = line[sig + 2..].rfind('<') else {
            return false;
        };
        if sig + 2 + params >= line.len() {
            return false;
        }
        let after = self.src[offset + 1..].trim_start();
        if after.starts_with('#') || after.starts_with("...") {
            return true;
        }
        let colon = after.find(':');
        let equals = after.find('=');
        let pipe = after.find('|');
        colon.is_some_and(|colon| {
            equals.is_none_or(|equals| colon < equals) && pipe.is_none_or(|pipe| colon < pipe)
        })
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
                    if offset < self.bytes.len() {
                        offset += 1;
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
