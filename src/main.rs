mod analysis;
mod cli;
mod coach;
mod ibt;
mod io;
mod live;
mod trace;
mod track;
mod types;
mod web;

use anyhow::{bail, Result};
use clap::Parser;

use crate::{
    analysis::summarize_session,
    cli::Cli,
    coach::{generate_feedback, speak_feedback},
    io::{collect_telemetry_files, parse_csv_laps, parse_ibt_laps},
    live::run_live_telemetry,
    web::run_ui_server,
};

fn main() -> Result<()> {
    let cli = Cli::parse();
    if let Some(input) = &cli.make_fixture {
        let summary = ibt::write_fixture(input, cli.fixture_out.as_deref(), cli.fixture_laps)?;
        println!("{summary}");
        return Ok(());
    }
    if cli.ui {
        let replay = cli.replay.clone().map(|path| (path, cli.replay_speed));
        return run_ui_server(cli.ui_port, cli.model.clone(), replay);
    }

    let mut laps = Vec::new();
    if cli.live {
        laps = run_live_telemetry(cli.live_duration_sec)?;
    } else {
        let telemetry_files = collect_telemetry_files(cli.file.as_ref(), cli.dir.as_ref())?;
        let mut failures = Vec::new();

        for file in telemetry_files {
            let extension = file.extension().and_then(|ext| ext.to_str()).unwrap_or("").to_ascii_lowercase();
            match extension.as_str() {
                "csv" => match parse_csv_laps(&file) {
                    Ok(parsed) => laps.extend(parsed),
                    Err(err) => failures.push(format!("{}: {err}", file.display())),
                },
                "ibt" => match parse_ibt_laps(&file) {
                    Ok(parsed) => laps.extend(parsed),
                    Err(err) => failures.push(format!("{}: {err}", file.display())),
                },
                _ => {}
            }
        }

        if !failures.is_empty() {
            eprintln!("Skipped {} telemetry file(s):", failures.len());
            for failure in failures {
                eprintln!("  - {failure}");
            }
        }
    }

    if laps.is_empty() {
        if cli.live {
            bail!(
                "No usable live lap data captured. Increase --live-duration-sec or start capture while driving in an active session."
            );
        }
        bail!("No usable telemetry data found. Pass --file <session.ibt|laps.csv> or --dir <telemetry-folder>.");
    }

    let summary = summarize_session(&laps);
    println!("Fastest lap: {:.3}s (lap {})", summary.fastest_lap_time_s, summary.fastest_lap);
    println!("Average lap: {:.3}s", summary.average_lap_time_s);
    if let (Some(name), Some(time)) = (&summary.slowest_sector_name, summary.slowest_sector_time_s) {
        println!("Focus sector: {name} ({time:.3}s on the fastest lap)");
    }
    if let Some(speed) = summary.average_speed_kph {
        println!("Average speed: {speed:.1} kph");
    }
    if let Some(temp) = summary.tyre_temp_avg_c {
        println!("Average tyre temp: {temp:.1}C");
    }
    println!("Suggestions:");
    for suggestion in &summary.suggestions {
        println!("  - {suggestion}");
    }

    let feedback = generate_feedback(&summary, &cli.model);
    println!("\nCoach feedback:\n");
    println!("{feedback}");

    if cli.talk {
        speak_feedback(&feedback, &cli.voice)?;
    }

    Ok(())
}
