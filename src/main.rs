use one2md::{convert_files, convert_files_with_hierarchy};
use std::env;
use std::ffi::OsString;
use std::path::PathBuf;
use std::process::ExitCode;

const HELP: &str = "\
Convert Microsoft OneNote files to a folder of Markdown pages.

Usage:
  one2md <INPUT>... [-o <OUTPUT>]

Arguments:
  <INPUT>...          One or more .one, .onetoc2, or .onepkg inputs

Options:
  -o, --output DIR    Output directory (default: INPUT without its extension)
  --source-root DIR   Preserve input parent folders relative to DIR
  -h, --help          Show this help
  -V, --version       Show the version
";

fn main() -> ExitCode {
    match run(env::args_os().skip(1)) {
        Ok(()) => ExitCode::SUCCESS,
        Err((code, message)) => {
            eprintln!("one2md: {message}");
            ExitCode::from(code)
        }
    }
}

fn run(args: impl Iterator<Item = OsString>) -> Result<(), (u8, String)> {
    let mut inputs = Vec::new();
    let mut output = None;
    let mut source_root = None;
    let mut args = args.peekable();

    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("-h" | "--help") => {
                print!("{HELP}");
                return Ok(());
            }
            Some("-V" | "--version") => {
                println!("one2md {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            Some("-o" | "--output") => {
                let value = args
                    .next()
                    .ok_or_else(|| (2, "--output requires a path".to_owned()))?;
                if output.replace(PathBuf::from(value)).is_some() {
                    return Err((2, "--output may only be specified once".to_owned()));
                }
            }
            Some("--source-root") => {
                let value = args
                    .next()
                    .ok_or_else(|| (2, "--source-root requires a path".to_owned()))?;
                if source_root.replace(PathBuf::from(value)).is_some() {
                    return Err((2, "--source-root may only be specified once".to_owned()));
                }
            }
            Some(value) if value.starts_with('-') => {
                return Err((2, format!("unknown option: {value}")));
            }
            _ => {
                inputs.push(PathBuf::from(arg));
            }
        }
    }

    if inputs.is_empty() {
        return Err((2, format!("missing input file\n\n{HELP}")));
    }
    let output = match output {
        Some(output) => output,
        None if inputs.len() == 1 => inputs[0].with_extension(""),
        None => {
            return Err((
                2,
                "--output is required when converting multiple inputs".to_owned(),
            ));
        }
    };

    let summary = match source_root {
        Some(source_root) => convert_files_with_hierarchy(&inputs, &source_root, &output),
        None => convert_files(&inputs, &output),
    }
    .map_err(|err| (1, err.to_string()))?;
    println!(
        "Converted {} page{} and {} asset{} to {}",
        summary.pages,
        plural(summary.pages),
        summary.assets,
        plural(summary.assets),
        summary.output.display()
    );

    for warning in &summary.warnings {
        eprintln!("warning: {warning}");
    }

    Ok(())
}

fn plural(count: usize) -> &'static str {
    if count == 1 { "" } else { "s" }
}
