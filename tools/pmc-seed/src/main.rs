#![forbid(unsafe_code)]

use std::process::ExitCode;

use pmc_seed::{seed_training, SeedOptions};

const USAGE: &str = "pmc-seed [--reset-training]\n\n\
Seeds the desktop's Training workspace with the public-safe demo scenario.\n\
Takes no path and no workspace kind: the Live workspace can never be seeded.\n\
--reset-training  delete a workspace this seed wrote before and rebuild it.";

fn main() -> ExitCode {
    let mut options = SeedOptions::default();
    for argument in std::env::args().skip(1) {
        match argument.as_str() {
            "--reset-training" => options.reset_training = true,
            "--help" | "-h" => {
                println!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            other => {
                eprintln!("unknown argument: {other}\n\n{USAGE}");
                return ExitCode::from(2);
            }
        }
    }
    match seed_training(options) {
        Ok(report) => {
            println!(
                "seeded Training workspace at {}",
                report.training_root.display()
            );
            println!(
                "schema {} · ledger revision {} · seed {} v{}",
                report.manifest.schema_version,
                report.manifest.ledger_revision,
                report.manifest.seed_id,
                report.manifest.seed_version
            );
            for (name, count) in &report.manifest.counts {
                println!("  {name}: {count}");
            }
            println!(
                "start the desktop with --pmc-workspace=training (debug builds only) to open it"
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("pmc-seed: {error}");
            ExitCode::FAILURE
        }
    }
}
