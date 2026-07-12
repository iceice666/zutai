use std::error::Error;

use clap::Parser;

#[derive(Parser)]
#[command(
    name = "zutai-web",
    about = "Build and serve Zutai browser applications",
    arg_required_else_help = true
)]
struct Cli {
    /// Filesystem root containing manifest.json and the Zutai stdlib modules.
    #[arg(long, global = true, env = "ZUTAI_STDLIB_ROOT")]
    stdlib_root: Option<std::path::PathBuf>,
    #[command(subcommand)]
    command: zutai_web::WebCommand,
}

fn main() -> Result<(), Box<dyn Error>> {
    let cli = Cli::parse();
    if let Some(root) = cli.stdlib_root {
        zutai_semantic::configure_stdlib_root(root)?;
    }
    cli.command.run()
}
