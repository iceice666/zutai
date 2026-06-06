pub mod decl;
pub mod expr;
pub mod lex;
pub mod pattern;
pub mod type_expr;

use winnow::Parser;
use winnow::combinator::eof;

use crate::ast::File;
use crate::error::ParseError;
use crate::span::Span;

use self::decl::parse_file;
use self::lex::ws;

pub fn parse(input: &str) -> Result<File, Vec<ParseError>> {
    lex::BASE_PTR.with(|c| c.set(input.as_ptr() as usize));
    lex::DEPTH.with(|c| c.set(0));
    let mut s = input;
    let result = (parse_file, ws, eof)
        .map(|(file, _, _)| file)
        .parse_next(&mut s);

    match result {
        Ok(file) => Ok(file),
        Err(e) => {
            let offset = input.len() - s.len();
            let span = Span::new(offset, offset);
            Err(vec![ParseError::new(span, e.to_string())])
        }
    }
}
