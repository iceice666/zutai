pub(crate) struct Cursor<'a> {
    src: &'a str,
    pos: usize,
}

impl<'a> Cursor<'a> {
    pub(crate) fn new(src: &'a str) -> Self {
        Self { src, pos: 0 }
    }

    pub(crate) fn pos(&self) -> usize {
        self.pos
    }

    pub(crate) fn is_eof(&self) -> bool {
        self.pos >= self.src.len()
    }

    pub(crate) fn peek(&self) -> Option<u8> {
        self.src.as_bytes().get(self.pos).copied()
    }

    pub(crate) fn peek_at(&self, offset: usize) -> Option<u8> {
        self.src.as_bytes().get(self.pos + offset).copied()
    }

    pub(crate) fn bump(&mut self) -> Option<u8> {
        let b = self.src.as_bytes().get(self.pos).copied();
        if b.is_some() {
            self.pos += 1;
        }
        b
    }

    /// Advance by one Unicode code point (used for non-ASCII error recovery).
    pub(crate) fn bump_char(&mut self) {
        if let Some(ch) = self.src[self.pos..].chars().next() {
            self.pos += ch.len_utf8();
        }
    }

    pub(crate) fn eat_while(&mut self, pred: impl Fn(u8) -> bool) {
        while let Some(b) = self.peek() {
            if pred(b) {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    pub(crate) fn slice(&self, start: usize) -> &'a str {
        &self.src[start..self.pos]
    }
}
