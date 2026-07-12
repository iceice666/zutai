use std::error::Error;

use clap::Parser;

#[derive(Parser)]
#[command(
    name = "zutai-web",
    about = "Build and serve Zutai browser applications",
    arg_required_else_help = true
)]
struct Cli {
    #[command(subcommand)]
    command: zutai_web::WebCommand,
}

fn main() -> Result<(), Box<dyn Error>> {
    Cli::parse().command.run()
}
