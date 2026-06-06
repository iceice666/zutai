use std::env;
use std::error::Error;
use std::fs;
use std::path::Path;

fn main() -> Result<(), Box<dyn Error>> {
    let path = parse_file_arg()?;
    let ext = extension_or_error(&path)?;
    let contents = fs::read_to_string(&path)?;

    match ext.as_str() {
        "zti" => {
            let ast =
                zutai_im::parse(&contents).map_err(|e| format!("Failed to parse .zti: {e}"))?;
            print_ast("zti", &ast);
        }
        "zt" => {
            let ast = zutai_syntax::parse(&contents).map_err(format_zt_errors)?;
            print_ast("zt", &ast);
        }
        other => return Err(format!("Unsupported extension: {other}").into()),
    }

    Ok(())
}

fn format_zt_errors(errs: Vec<zutai_syntax::ParseError>) -> Box<dyn Error> {
    let mut s = String::from("Failed to parse .zt:");
    for e in errs {
        s.push_str(&format!("\n  {e}"));
    }
    s.into()
}

fn parse_file_arg() -> Result<String, Box<dyn Error>> {
    let mut args = env::args().skip(1);
    let path = args.next().ok_or("usage: zutai-cli <path>")?;

    if args.next().is_some() {
        return Err("usage: zutai-cli <path>".into());
    }

    Ok(path)
}

fn extension_or_error(path: &str) -> Result<String, Box<dyn Error>> {
    let extension = Path::new(path)
        .extension()
        .and_then(|ext| ext.to_str())
        .ok_or_else(|| format!("File has no extension: {path}"))?
        .to_ascii_lowercase();

    Ok(extension)
}

fn print_ast(label: &str, ast: &impl std::fmt::Display) {
    println!("Parsed .{label} AST:");
    println!("{ast}");
}
