use clap::{Parser, ValueEnum};
use lu_common::exit::ExitCode;
use lu_common::format::{FormatWriter, OutputFormat, Record};
use lu_query::engine::{self, Engine};
use std::io;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "lu-query", about = "Logic knowledge base query engine")]
struct Cli {
    /// Query to evaluate, e.g. "stale(X)" or "depends(main_o, X)"
    query: String,

    /// Knowledge base file(s)
    #[arg(short, long)]
    kb: Vec<PathBuf>,

    /// Add an inline fact: "pred(a, b)"
    #[arg(long)]
    fact: Vec<String>,

    /// Return all solutions (default: first only)
    #[arg(long)]
    all: bool,

    /// Use external engine binary instead of built-in
    #[arg(long)]
    engine: Option<String>,

    /// Query timeout in seconds
    #[arg(long)]
    timeout: Option<u64>,

    /// Output format
    #[arg(long, default_value = "plain")]
    format: FormatArg,

    /// Print protocol version and exit
    #[arg(long)]
    protocol_version: bool,
}

#[derive(Clone, ValueEnum)]
enum FormatArg {
    Plain,
    Json,
    Tsv,
    Csv,
    Toml,
    Shell,
}

impl From<FormatArg> for OutputFormat {
    fn from(f: FormatArg) -> Self {
        match f {
            FormatArg::Plain => OutputFormat::Plain,
            FormatArg::Json => OutputFormat::Json,
            FormatArg::Tsv => OutputFormat::Tsv,
            FormatArg::Csv => OutputFormat::Csv,
            FormatArg::Toml => OutputFormat::Toml,
            FormatArg::Shell => OutputFormat::Shell,
        }
    }
}

fn main() -> std::process::ExitCode {
    let cli = Cli::parse();

    if cli.protocol_version {
        println!("0.1.0");
        return ExitCode::Success.into();
    }

    // External engine delegation
    if let Some(ref engine_path) = cli.engine {
        return delegate_to_external(engine_path, &cli);
    }

    let mut eng = Engine::new();

    // Load KB files
    for kb_path in &cli.kb {
        let source = match std::fs::read_to_string(kb_path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("lu-query: cannot read {}: {e}", kb_path.display());
                return ExitCode::Error.into();
            }
        };
        match lu_query::load_kb(&source) {
            Ok(module) => eng.load_module(&module),
            Err(e) => {
                eprintln!("lu-query: {}: {e}", kb_path.display());
                return ExitCode::Error.into();
            }
        }
    }

    // Add inline facts
    for fact_str in &cli.fact {
        match engine::parse_query(fact_str) {
            Ok((name, args)) => {
                let values: Vec<engine::Value> = args
                    .into_iter()
                    .map(|a| match a {
                        engine::QueryArg::Bound(v) => v,
                        engine::QueryArg::Var(n) => engine::Value::Atom(n),
                    })
                    .collect();
                eng.add_fact(&name, values);
            }
            Err(e) => {
                eprintln!("lu-query: invalid fact '{fact_str}': {e}");
                return ExitCode::Error.into();
            }
        }
    }

    // Parse query
    let (query_name, query_args) = match engine::parse_query(&cli.query) {
        Ok(q) => q,
        Err(e) => {
            eprintln!("lu-query: {e}");
            return ExitCode::Error.into();
        }
    };

    // Execute query
    let results = eng.query(&query_name, &query_args);

    if results.is_empty() {
        return ExitCode::Failure.into();
    }

    let out_format: OutputFormat = cli.format.into();
    let stdout = io::stdout();
    let mut writer = FormatWriter::new(stdout.lock(), out_format);

    let results_to_show = if cli.all {
        &results[..]
    } else {
        &results[..1]
    };

    // Determine variable names from query args
    let var_names: Vec<String> = query_args
        .iter()
        .filter_map(|a| match a {
            engine::QueryArg::Var(n) => Some(n.clone()),
            _ => None,
        })
        .collect();

    for binding in results_to_show {
        let mut rec = Record::new();
        for name in &var_names {
            if let Some(val) = binding.get(name) {
                rec = rec.field(name.as_str(), &val.to_string());
            }
        }
        if let Err(e) = writer.write_record(&rec) {
            eprintln!("lu-query: {e}");
            return ExitCode::Error.into();
        }
    }

    let _ = writer.flush();
    ExitCode::Success.into()
}

fn delegate_to_external(engine_path: &str, cli: &Cli) -> std::process::ExitCode {
    let mut cmd = std::process::Command::new(engine_path);
    for kb in &cli.kb {
        cmd.arg("--kb").arg(kb);
    }
    for fact in &cli.fact {
        cmd.arg("--fact").arg(fact);
    }
    if cli.all {
        cmd.arg("--all");
    }
    cmd.arg(&cli.query);

    match cmd.status() {
        Ok(status) => std::process::ExitCode::from(status.code().unwrap_or(2) as u8),
        Err(e) => {
            eprintln!("lu-query: cannot run engine '{engine_path}': {e}");
            ExitCode::Error.into()
        }
    }
}
