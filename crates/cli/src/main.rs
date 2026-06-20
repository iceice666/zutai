use std::error::Error;

use clap::Parser;

mod commands;
mod diagnostics;

fn main() -> Result<(), Box<dyn Error>> {
    let cli = Cli::parse();
    match cli.command {
        Some(Commands::Run { path }) => commands::run_file(&path)?,
        Some(Commands::Parse { path }) => commands::run_parse(&path)?,
        Some(Commands::Check { path }) => commands::run_check(&path)?,
        Some(Commands::Compile { path, output }) => {
            commands::run_compile(&path, output.as_deref())?;
        }
        Some(Commands::Dataflow { path }) => commands::run_dataflow(&path)?,
        Some(Commands::Repl) => commands::run_repl()?,
        None => {
            let path = cli.path.expect("clap requires a subcommand or path");
            commands::run_bare_path(&path)?;
        }
    }
    Ok(())
}

// ─── CLI definition ──────────────────────────────────────────────────────────

#[derive(Parser)]
#[command(
    name = "zutai-cli",
    about = "Zutai language compiler and interpreter",
    arg_required_else_help = true
)]
struct Cli {
    /// Legacy shorthand: run .zt files or parse .zti files without a subcommand.
    path: Option<String>,
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(clap::Subcommand)]
enum Commands {
    /// Evaluate a .zt file and print the result
    Run {
        /// Path to the .zt file
        path: String,
    },
    /// Parse a file and print the AST
    Parse {
        /// Path to the .zt or .zti file
        path: String,
    },
    /// Type-check a .zt file and print diagnostics
    Check {
        /// Path to the .zt file
        path: String,
    },
    /// Compile a .zt file to LLVM IR
    Compile {
        /// Path to the .zt file
        path: String,
        /// Output file path (default: stdout)
        #[arg(short)]
        output: Option<String>,
    },
    /// Print the Dataflow Core graph for a .zt file
    Dataflow {
        /// Path to the .zt file
        path: String,
    },
    /// Run an interactive REPL
    Repl,
}
