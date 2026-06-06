use std::env;
use std::error::Error;
use std::fs;
use std::path::Path;

fn main() -> Result<(), Box<dyn Error>> {
    let path = parse_file_arg()?;
    let ext = extension_or_error(&path)?;
    let contents = fs::read_to_string(&path)?;

    let ast = match ext.as_str() {
        "zti" => parse_immediate(&contents),
        "zt" => parse_general(&contents),
        other => return Err(format!("Unsupported extension: {other}").into()),
    }?;

    print_ast(&ext, &ast);

    Ok(())
}

fn parse_general(_input: &str) -> Result<zutai_types::Block, Box<dyn Error>> {
    unimplemented!("General mode parser is intentionally unimplemented")
}

fn parse_immediate(input: &str) -> Result<zutai_types::Block, Box<dyn Error>> {
    zutai_im::parse(input).map_err(|err| format!("Failed to parse .zti: {err}").into())
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
