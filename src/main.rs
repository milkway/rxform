use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;

/// Convert an XLSForm spreadsheet (.xlsx/.xls/.ods) to an ODK XForm XML file.
#[derive(Parser)]
#[command(name = "rxform", version, about)]
struct Cli {
    /// Path to the XLSForm workbook
    input: PathBuf,

    /// Output XForm path (defaults to the input path with a .xml extension)
    #[arg(short, long)]
    output: Option<PathBuf>,

    /// Print the XForm to stdout instead of writing a file
    #[arg(long)]
    stdout: bool,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let conversion = match rxform::convert(&cli.input) {
        Ok(conversion) => conversion,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };
    if cli.stdout {
        print!("{}", conversion.xml);
        if conversion.itemsets_csv.is_some() {
            eprintln!("note: this form uses select_one_external; run without --stdout to also write itemsets.csv");
        }
        return ExitCode::SUCCESS;
    }
    let output = cli
        .output
        .unwrap_or_else(|| cli.input.with_extension("xml"));
    if let Err(e) = std::fs::write(&output, conversion.xml) {
        eprintln!("error writing {}: {e}", output.display());
        return ExitCode::FAILURE;
    }
    println!("{}", output.display());
    if let Some(csv) = conversion.itemsets_csv {
        let csv_path = output.with_file_name("itemsets.csv");
        if let Err(e) = std::fs::write(&csv_path, csv) {
            eprintln!("error writing {}: {e}", csv_path.display());
            return ExitCode::FAILURE;
        }
        println!("{}", csv_path.display());
    }
    ExitCode::SUCCESS
}
