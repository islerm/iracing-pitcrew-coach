mod analysis;
mod cli;
mod coach;
mod server;
mod stats;
mod telemetry;
mod voice;

use anyhow::{Context, Result};
use clap::Parser;

use crate::cli::Cli;

/// Serves the coaching UI, or with `--make-fixture` writes anonymised test data and exits.
fn main() -> Result<()> {
    let cli = Cli::parse();
    if let Some(input) = &cli.make_fixture {
        let summary = telemetry::ibt::write_fixture(input, cli.fixture_out.as_deref(), cli.fixture_laps)?;
        println!("{summary}");
        return Ok(());
    }
    // Resolve before a possible change of directory below.
    let replay = cli.replay.map(|path| std::path::absolute(path).map(|path| (path, cli.replay_speed))).transpose()?;
    enter_project_root()?;
    server::run_ui_server(cli.ui_port, cli.model, replay, cli.tts, cli.voice)
}

/// The UI, voice scripts and data folders are found relative to the working directory. When
/// `web/index.html` isn't there, move to the nearest folder above the executable that has it.
fn enter_project_root() -> Result<()> {
    if std::path::Path::new("web/index.html").is_file() {
        return Ok(());
    }
    let exe = std::env::current_exe().context("can't locate the executable")?;
    let root = exe
        .ancestors()
        .skip(1)
        .find(|dir| dir.join("web/index.html").is_file())
        .context("can't find web/index.html: run from the project folder, or keep the executable inside it")?;
    std::env::set_current_dir(root).with_context(|| format!("can't switch to {}", root.display()))
}
