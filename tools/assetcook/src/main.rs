//! Thin command-line front end over `somnium_asset::cook`.

use somnium_asset::cook::{CookConfig, CookPlan, default_cook_deadline, submit_cook};
use somnium_jobs::{JobPriority, JobSystem};
use std::{env, fs, path::PathBuf, process::ExitCode};

const PLAN_VERSION: u32 = 1;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("assetcook: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let arguments: Vec<_> = env::args_os().skip(1).collect();
    if arguments.len() != 4 {
        return Err(
            "usage: somnium_assetcook <source-root> <output-root> <cache-root> <plan.json>".into(),
        );
    }
    let source_root = PathBuf::from(&arguments[0]);
    let output_root = PathBuf::from(&arguments[1]);
    let cache_root = PathBuf::from(&arguments[2]);
    let plan_path = PathBuf::from(&arguments[3]);
    let plan: CookPlan = serde_json::from_slice(
        &fs::read(&plan_path).map_err(|error| format!("read {}: {error}", plan_path.display()))?,
    )
    .map_err(|error| format!("parse {}: {error}", plan_path.display()))?;
    if plan.version > PLAN_VERSION {
        return Err(format!(
            "cook plan version {} is newer than this tool",
            plan.version
        ));
    }
    let config = CookConfig {
        source_root,
        output_root,
        cache_root,
        cooker_version: plan.cooker_version,
    };
    // The CLI has no frame loop, but still declares the same priority and
    // deadline the editor uses. Inline execution keeps diagnostics and output
    // ordering deterministic; editor callers use their process JobSystem.
    let mut jobs = JobSystem::single_threaded();
    let handle = submit_cook(
        &mut jobs,
        config,
        plan.assets,
        JobPriority::User,
        default_cook_deadline(),
    )
    .map_err(|error| format!("submit cook: {error:?}"))?;
    let report = handle
        .try_take()
        .ok_or_else(|| "inline cook returned no result".to_string())?
        .map_err(|error| format!("cook job: {error:?}"))?;
    let cooked = report
        .status
        .values()
        .filter(|status| **status == somnium_asset::cook::CookStatus::Cooked)
        .count();
    let cached = report.status.len() - cooked;
    println!(
        "assetcook: {} assets ({} cooked, {} cached)",
        report.status.len(),
        cooked,
        cached
    );
    Ok(())
}
