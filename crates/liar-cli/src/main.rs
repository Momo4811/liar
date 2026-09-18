//! Static analysis for Python: the command line interface.

#![forbid(unsafe_code)]

mod config;
mod discover;
mod render;

use clap::{Parser, Subcommand};
use config::Config;
use liar_core::analysis::analyse;
use liar_core::ast::{Ast, parse};
use liar_core::ids::FileId;
use liar_core::messages::{MessageTable, Tone};
use liar_core::source::SourceMap;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser)]
#[command(
    name = "liar",
    version,
    about = "Finds code that says one thing and does another"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Analyse files or directories
    Check {
        /// Files or directories to analyse
        #[arg(required = true)]
        paths: Vec<PathBuf>,

        /// professional, dry, or brutal
        #[arg(long)]
        tone: Option<String>,

        /// Path to liar.toml
        #[arg(long)]
        config: Option<PathBuf>,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        // The usual convention for a linter: 0 clean, 1 findings, 2 the tool
        // itself failed. A build script can then tell "your code has problems"
        // from "liar could not run".
        Ok(true) => ExitCode::from(1),
        Ok(false) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("liar: {message}");
            ExitCode::from(2)
        }
    }
}

/// Returns whether any findings were reported.
fn run(cli: Cli) -> Result<bool, String> {
    let Commands::Check {
        paths,
        tone,
        config,
    } = cli.command;

    let mut settings = Config::load(config.as_deref()).map_err(|e| e.to_string())?;
    if let Some(tone) = tone {
        settings.tone = tone.parse::<Tone>().map_err(|e| e.to_string())?;
    }

    let files = discover::discover(&paths, &settings.exclude).map_err(|e| e.to_string())?;

    let mut sources = SourceMap::new();
    let mut asts: BTreeMap<FileId, Ast> = BTreeMap::new();

    for path in files {
        let text = std::fs::read_to_string(&path)
            .map_err(|e| format!("could not read {}: {e}", path.display()))?;
        let file_id = sources.add(path.clone(), text);

        let ast = parse(sources.get(file_id).text()).map_err(|e| {
            let position = sources.get(file_id).position(e.span.start);
            format!(
                "{}:{}:{}: {}",
                path.display(),
                position.line,
                position.column,
                e.message
            )
        })?;
        asts.insert(file_id, ast);
    }

    // Analysed as one project rather than file by file, so a name imported
    // from another file resolves to its real definition.
    let mut findings = analyse(&sources, &asts);

    // Filtering here rather than inside each check means a disabled check
    // costs nothing to add and cannot leak a finding by forgetting to ask.
    findings.retain(|finding| settings.is_enabled(finding.check));

    if !findings.is_empty() {
        print!(
            "{}",
            render::render(&findings, &sources, MessageTable::embedded(), settings.tone)
        );
    }

    Ok(!findings.is_empty())
}
