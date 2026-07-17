use std::error::Error;

use clap::{CommandFactory, Parser, ValueEnum, error::ErrorKind};

mod commands;
mod diagnostics;
mod lsp;

fn main() -> Result<(), Box<dyn Error>> {
    let cli = Cli::parse();
    if let Some(root) = cli.stdlib_root {
        zutai_semantic::configure_stdlib_root(root)?;
    }
    match cli.command {
        Some(Commands::Run { path }) => commands::run_file(&path)?,
        Some(Commands::Parse { path }) => commands::run_parse(&path)?,
        Some(Commands::Json { path }) => commands::run_json(&path)?,
        Some(Commands::Check { path }) => commands::run_check(&path)?,
        Some(Commands::Compile {
            path,
            output,
            emit,
            metadata,
        }) => {
            commands::run_compile(&path, output.as_deref(), emit.into(), metadata.as_deref())?;
        }
        Some(Commands::Dataflow { path }) => commands::run_dataflow(&path)?,
        Some(Commands::Web { command }) => command.run()?,
        Some(Commands::Package { command }) => command.run()?,
        Some(Commands::Repl) => commands::run_repl()?,
        Some(Commands::Lsp) => lsp::run()?,
        None => match cli.path {
            Some(path) => commands::run_bare_path(&path)?,
            None => Cli::command()
                .error(
                    ErrorKind::MissingRequiredArgument,
                    "a subcommand or path is required",
                )
                .exit(),
        },
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
    /// Filesystem root containing zutai.zti and the Zutai stdlib packages.
    #[arg(long, global = true, env = "ZUTAI_STDLIB_ROOT")]
    stdlib_root: Option<std::path::PathBuf>,
    /// Legacy shorthand: run .zt files or parse .zti files without a subcommand.
    path: Option<String>,
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum CompileEmit {
    Llvm,
    Obj,
    Bin,
    Lib,
}

impl From<CompileEmit> for commands::EmitMode {
    fn from(value: CompileEmit) -> Self {
        match value {
            CompileEmit::Llvm => commands::EmitMode::Llvm,
            CompileEmit::Obj => commands::EmitMode::Obj,
            CompileEmit::Bin => commands::EmitMode::Bin,
            CompileEmit::Lib => commands::EmitMode::Lib,
        }
    }
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
    /// Parse .zti or evaluate .zt and print the final result as JSON
    Json {
        /// Path to the .zt or .zti file
        path: String,
    },
    /// Type-check a .zt file and print diagnostics
    Check {
        /// Path to the .zt file
        path: String,
    },
    /// Compile a .zt file
    Compile {
        /// Path to the .zt file
        path: String,
        /// Output file path (default: stdout for LLVM, derived path for native artifacts)
        #[arg(short)]
        output: Option<String>,
        /// Artifact to emit
        #[arg(long, value_enum, default_value_t = CompileEmit::Llvm)]
        emit: CompileEmit,
        /// Write deterministic build metadata as JSON
        #[arg(long, value_name = "PATH")]
        metadata: Option<std::path::PathBuf>,
    },
    /// Print the Dataflow Core graph for a .zt file
    Dataflow {
        /// Path to the .zt file
        path: String,
    },
    /// Build or serve a whole-document Zutai browser application
    Web {
        #[command(subcommand)]
        command: zutai_web::WebCommand,
    },
    /// Prepare and maintain a locked package graph
    Package {
        #[command(subcommand)]
        command: commands::PackageCommand,
    },
    /// Run an interactive REPL
    Repl,
    /// Start the Language Server Protocol service on standard input/output
    Lsp,
}
