use std::env;
use std::error::Error;
use std::fs;
use std::path::Path;

fn main() -> Result<(), Box<dyn Error>> {
    let path = parse_file_arg()?;
    let contents = fs::read_to_string(&path)?;

    match Path::new(&path).extension().and_then(|e| e.to_str()) {
        Some("zti") => {
            let ast =
                zutai_im::parse(&contents).map_err(|err| format!("Failed to parse .zti: {err}"))?;
            println!("Parsed .zti AST:");
            println!("{ast}");
        }
        Some("zt") => {
            parse_zt(&path, &contents);
        }
        other => return Err(format!("Unsupported extension: {:?}", other).into()),
    }
    Ok(())
}

fn parse_zt(path: &str, src: &str) {
    let result = zutai_syntax::parse(src);

    // Print the lossless CST.
    println!("{:#?}", result.syntax());

    // Run semantic analysis and merge all diagnostics.
    let semantic = zutai_semantic::analyze(&result.syntax());
    let mut all_diags = result.diagnostics;
    all_diags.extend(semantic.diagnostics);

    if all_diags.is_empty() {
        println!("No diagnostics — parsed and analyzed successfully.");
    } else {
        eprintln!("{} diagnostic(s):", all_diags.len());
        zutai_syntax::diag::render::eprint_diagnostics(&all_diags, path, src);
    }
}

fn parse_file_arg() -> Result<String, Box<dyn Error>> {
    let mut args = env::args().skip(1);
    let path = args.next().ok_or("usage: zutai-cli <path>")?;
    if args.next().is_some() {
        return Err("usage: zutai-cli <path>".into());
    }
    Ok(path)
}
