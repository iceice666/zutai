use std::error::Error;
use std::path::PathBuf;

use clap::{Parser, ValueEnum};

mod commands;
mod diagnostics;
mod lsp;

fn main() -> Result<(), Box<dyn Error>> {
    let cli = Cli::parse();
    match cli.command {
        Some(Commands::Run { path }) => commands::run_file(&path)?,
        Some(Commands::Parse { path }) => commands::run_parse(&path)?,
        Some(Commands::Json { path }) => commands::run_json(&path)?,
        Some(Commands::Check { path }) => commands::run_check(&path)?,
        Some(Commands::Compile { path, output, emit }) => {
            commands::run_compile(&path, output.as_deref(), emit.into())?;
        }
        Some(Commands::Dataflow { path }) => commands::run_dataflow(&path)?,
        Some(Commands::Web { command }) => match command {
            WebCommands::Build {
                entry,
                out_dir,
                source_root,
                public_dir,
            } => commands::web::run_web_build(commands::web::WebBuildOptions {
                entry,
                out_dir,
                source_root,
                public_dir,
            })?,
            WebCommands::Serve {
                entry,
                out_dir,
                source_root,
                public_dir,
                addr,
                no_build,
            } => commands::web::run_web_serve(
                commands::web::WebBuildOptions {
                    entry,
                    out_dir,
                    source_root,
                    public_dir,
                },
                &addr,
                no_build,
            )?,
        },
        Some(Commands::Repl) => commands::run_repl()?,
        Some(Commands::Lsp) => lsp::run()?,
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
    },
    /// Print the Dataflow Core graph for a .zt file
    Dataflow {
        /// Path to the .zt file
        path: String,
    },
    /// Build or serve a whole-document Zutai browser application
    Web {
        #[command(subcommand)]
        command: WebCommands,
    },
    /// Run an interactive REPL
    Repl,
    /// Start the Language Server Protocol service on standard input/output
    Lsp,
}

#[derive(clap::Subcommand)]
enum WebCommands {
    /// Build a prerendered static site and its interpreter WebAssembly kernel
    Build {
        /// Browser program entry `.zt` file
        entry: PathBuf,
        /// Static output directory
        #[arg(short = 'o', long, default_value = "dist")]
        out_dir: PathBuf,
        /// Root used for portable source paths (defaults to the entry directory)
        #[arg(long)]
        source_root: Option<PathBuf>,
        /// Static assets copied verbatim (defaults to `<source-root>/public`)
        #[arg(long)]
        public_dir: Option<PathBuf>,
    },
    /// Build, watch, and serve with full-page reload on successful changes
    Serve {
        /// Browser program entry `.zt` file
        entry: PathBuf,
        /// Static output directory
        #[arg(short = 'o', long, default_value = "dist")]
        out_dir: PathBuf,
        /// Root used for portable source paths (defaults to the entry directory)
        #[arg(long)]
        source_root: Option<PathBuf>,
        /// Static assets copied verbatim (defaults to `<source-root>/public`)
        #[arg(long)]
        public_dir: Option<PathBuf>,
        /// Address for the development server
        #[arg(long, default_value = "127.0.0.1:8787")]
        addr: String,
        /// Serve the existing output directory without rebuilding first
        #[arg(long)]
        no_build: bool,
    },
}
