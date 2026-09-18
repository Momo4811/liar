//! Static analysis for Python: the command line interface.

#![forbid(unsafe_code)]

mod config;
mod discover;
mod render;

use clap::{Parser, Subcommand};
use config::Config;
use liar_core::analysis::analyse;
use liar_core::ast::{Ast, Expr, parse};
use liar_core::ids::FileId;
use liar_core::index::{Index, IndexInput};
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

    /// Ask the engine what it knows
    #[command(subcommand)]
    Debug(DebugCommands),
}

#[derive(Subcommand)]
enum DebugCommands {
    /// Print what a name at a position resolves to
    ///
    /// Takes a location as FILE:LINE:COLUMN. The directory containing the file
    /// is analysed as the project, so imports resolve the way they would in a
    /// real run.
    Resolve {
        /// FILE:LINE:COLUMN, for example src/app.py:12:9
        location: String,
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
    match cli.command {
        Commands::Check {
            paths,
            tone,
            config,
        } => check(paths, tone, config),
        Commands::Debug(DebugCommands::Resolve { location }) => {
            resolve(&location)?;
            Ok(false)
        }
    }
}

/// Loads every Python file under `root` and parses what it can.
fn load(root: &std::path::Path) -> Result<(SourceMap, BTreeMap<FileId, Ast>), String> {
    let files = discover::discover(&[root.to_path_buf()], &[]).map_err(|e| e.to_string())?;

    let mut sources = SourceMap::new();
    let mut asts = BTreeMap::new();

    for path in files {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let id = sources.add(path, text);
        // A file that does not parse is skipped rather than fatal: the point is
        // to answer a question about one position, and one broken neighbour
        // should not prevent that.
        if let Ok(ast) = parse(sources.get(id).text()) {
            asts.insert(id, ast);
        }
    }

    Ok((sources, asts))
}

/// Prints what the name at a position resolves to.
///
/// Without this, every checker is debugged by guesswork: you see a finding you
/// did not expect and have no way to ask the engine what it thought a name was.
fn resolve(location: &str) -> Result<(), String> {
    let (path, line, column) = parse_location(location)?;

    let root = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(&path);
    let (sources, asts) = load(root)?;

    let canonical = std::fs::canonicalize(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let (file, source) = sources
        .iter()
        .find(|(_, source)| std::fs::canonicalize(source.path()).is_ok_and(|p| p == canonical))
        .ok_or_else(|| format!("{} was not among the analysed files", path.display()))?;

    let offset = source.offset_at(line, column).ok_or_else(|| {
        format!(
            "{}:{line}:{column} is not a position in the file",
            path.display()
        )
    })?;

    let ast = asts
        .get(&file)
        .ok_or_else(|| format!("{} did not parse", path.display()))?;

    let inputs: Vec<IndexInput<'_>> = asts
        .iter()
        .map(|(&id, ast)| IndexInput {
            file: id,
            path: sources.get(id).path(),
            ast,
        })
        .collect();
    let index = Index::build(&inputs);

    // The smallest name containing the offset is the one under the cursor.
    let found = ast
        .exprs()
        .filter_map(|(_, expr)| match expr {
            Expr::Name { name, span } if span.contains(offset) => Some((name, *span)),
            _ => None,
        })
        .min_by_key(|(_, span)| span.len());

    let Some((name, span)) = found else {
        println!("no name at {}:{line}:{column}", path.display());
        return Ok(());
    };

    // Likewise the smallest statement containing it gives the scope it sits in.
    let file_index = index
        .file(file)
        .ok_or_else(|| "the file was not indexed".to_string())?;
    let scope = ast
        .stmts()
        .filter(|(id, _)| {
            file_index.stmt_scope.contains_key(id) && ast.stmt_span(*id).contains(offset)
        })
        .min_by_key(|(id, _)| ast.stmt_span(*id).len())
        .map(|(id, _)| file_index.scope_of(id))
        .unwrap_or_else(|| file_index.scopes.root());

    println!("name:   {name}");
    println!("at:     {}:{line}:{column}", path.display());
    println!("span:   {}..{}", span.start, span.end);
    println!("scope:  {:?}", file_index.scopes.scope(scope).kind);

    match index.resolve(file, scope, name) {
        None => println!("\nresolves to: nothing - the engine says nothing about this name"),
        Some(resolved) => {
            let defining = sources.get(resolved.file);
            let position = defining.position(resolved.binding.name_span.start);
            println!("\nresolves to: {:?}", resolved.binding.kind);
            println!(
                "defined at:  {}:{}:{}",
                defining.path().display(),
                position.line,
                position.column
            );
            if let Some(dotted) = resolved.binding.dotted_path() {
                println!("dotted path: {dotted}");
            }
        }
    }

    Ok(())
}

fn parse_location(location: &str) -> Result<(PathBuf, u32, u32), String> {
    let bad = || format!("expected FILE:LINE:COLUMN, got {location:?}");

    // Split from the right, so a Windows drive letter in the path survives.
    let (rest, column) = location.rsplit_once(':').ok_or_else(bad)?;
    let (path, line) = rest.rsplit_once(':').ok_or_else(bad)?;

    Ok((
        PathBuf::from(path),
        line.parse().map_err(|_| bad())?,
        column.parse().map_err(|_| bad())?,
    ))
}

fn check(
    paths: Vec<PathBuf>,
    tone: Option<String>,
    config: Option<PathBuf>,
) -> Result<bool, String> {
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
