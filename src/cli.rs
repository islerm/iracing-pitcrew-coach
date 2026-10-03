use clap::Parser;
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(author, version, about = "Local iRacing pit-crew coach")]
pub struct Cli {
    #[arg(long, default_value_t = 8787, help = "Port for the web UI server")]
    pub ui_port: u16,

    #[arg(long, default_value = "llama3.2", help = "Ollama model to use for coaching")]
    pub model: String,

    #[arg(long, value_enum, default_value = "piper", help = "Voice engine for the coach: piper (fast), kokoro (more natural, slower) or espeak")]
    pub tts: crate::voice::Tts,

    #[arg(
        long,
        help = "Voice name for the engine. Piper: en_GB-cori-high (default), en_GB-jenny_dioco-medium, en_GB-alba-medium. Kokoro: bf_emma (default), bf_isabella, bm_george"
    )]
    pub voice: Option<String>,

    #[arg(
        long,
        value_name = "IBT",
        help = "'Start recording' replays this .ibt as if it were live iRacing (no sim needed)"
    )]
    pub replay: Option<PathBuf>,

    #[arg(long, default_value_t = 4.0, help = "Playback speed for --replay (1 = real time)")]
    pub replay_speed: f64,

    #[arg(long, help = "Don't start and save recordings automatically when you get in and out of the car")]
    pub no_auto_record: bool,

    #[arg(
        long,
        value_name = "IBT",
        help = "Write an anonymized, trimmed copy of this .ibt for use as test data, then exit"
    )]
    pub make_fixture: Option<PathBuf>,

    #[arg(long, default_value_t = 4, help = "With --make-fixture: number of timed laps to keep")]
    pub fixture_laps: usize,

    #[arg(long, value_name = "FILE", help = "With --make-fixture: output path (default tests/fixtures/<track>.ibt)")]
    pub fixture_out: Option<PathBuf>,
}
