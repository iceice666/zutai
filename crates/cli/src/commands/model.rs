use super::*;

use zutai_model::{CheckOptions, CheckOutcome, PassedKind, ScenarioReport};

/// Exhaustively model-check a `.zt` transition-system model.
///
/// Reuses the shared CLI analysis path (parse/HIR/THIR diagnostics exit early
/// with the standard rendering), then runs the bounded explicit-state engine.
/// Function-bearing model records are intentionally valid here even though they
/// are rejected by `run`/native ABI output, so `unsupported_cli_entry_type_reason`
/// is deliberately not consulted.
pub(crate) fn run_model_check(path: &str, max_states: usize) -> Result<(), Box<dyn Error>> {
    let contents = fs::read_to_string(path)?;
    let base = Path::new(path).parent();
    let cache = zutai_semantic::AnalysisCache::default();
    let analysis = super::compile::analyze_with_cli_diagnostics(path, &contents, base, &cache);
    if !analysis.is_thir_complete() {
        eprintln!("model-check error: model program is incomplete or not runnable");
        std::process::exit(1);
    }

    match zutai_model::check_analysis(&analysis, CheckOptions { max_states }) {
        Ok(CheckOutcome::Passed { scenarios }) => {
            for report in &scenarios {
                println!("{}", render_report(report));
            }
            println!("model-check: all {} scenarios passed", scenarios.len());
        }
        Ok(CheckOutcome::Failed { completed, message }) => {
            for report in &completed {
                println!("{}", render_report(report));
            }
            println!("{message}");
            std::process::exit(1);
        }
        Ok(CheckOutcome::Inconclusive {
            completed,
            scenario,
            visited,
        }) => {
            for report in &completed {
                println!("{}", render_report(report));
            }
            println!(
                "model-check: inconclusive: state limit reached (visited {visited} states) in scenario \"{scenario}\""
            );
            std::process::exit(2);
        }
        Err(error) => {
            eprintln!("model-check error: {error}");
            std::process::exit(1);
        }
    }
    Ok(())
}

/// Render a completed passing scenario's fixed one-line summary.
fn render_report(report: &ScenarioReport) -> String {
    match &report.kind {
        PassedKind::Safe => {
            format!(
                "scenario \"{}\": ok ({} states)",
                report.name, report.visited
            )
        }
        PassedKind::ExpectedViolation { property } => {
            format!(
                "scenario \"{}\": ok (violated \"{property}\" as expected)",
                report.name
            )
        }
    }
}
