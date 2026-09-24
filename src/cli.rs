use clap::Parser;
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(author, version, about = "Local iRacing pit-crew coach")]
pub struct Cli {
    #[arg(long, value_name = "FILE", help = "An .ibt telemetry file or a lap CSV to summarize")]
    pub file: Option<PathBuf>,

    #[arg(
        long,
        value_name = "DIR",
        help = "Summarize every .ibt/.csv in this folder together (default: the iRacing telemetry folder)"
    )]
    pub dir: Option<PathBuf>,

    #[arg(long, help = "Connect to live iRacing telemetry instead of reading recorded files")]
    pub live: bool,

    #[arg(long, default_value_t = 15, help = "How long to sample live telemetry, in seconds")]
    pub live_duration_sec: u64,

    #[arg(long, help = "Start the local web UI server")]
    pub ui: bool,

    #[arg(long, default_value_t = 8787, help = "Port for the web UI server")]
    pub ui_port: u16,

    #[arg(long, default_value = "llama3.2", help = "Ollama model to use for coaching")]
    pub model: String,

    #[arg(long, help = "Speak the summary out loud with TTS if available")]
    pub talk: bool,

    #[arg(long, default_value = "en-us", help = "Voice to use with espeak-ng or espeak")]
    pub voice: String,

    #[arg(
        long,
        value_name = "IBT",
        help = "With --ui: 'Start recording' replays this .ibt as if it were live iRacing (no sim needed)"
    )]
    pub replay: Option<PathBuf>,

    #[arg(long, default_value_t = 4.0, help = "Playback speed for --replay (1 = real time)")]
    pub replay_speed: f64,

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
