//! The coach's voice: local neural TTS (Piper or Kokoro) with an espeak fallback, through the
//! radio effect.
//!
//! The neural engines run in a long-lived Python sidecar (`voice/piper_tts.py` or
//! `voice/kokoro_tts.py`, started through `uv` when available) so the model is loaded once.
//! Requests and replies are JSON lines. Piper is the default: it's several times faster than
//! Kokoro, which sounds a little more natural.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

use anyhow::{anyhow, bail, Context, Result};
use serde::Serialize;

pub mod radio;
pub mod speech;

use crate::voice::radio::{radio_filter, read_wav, write_wav, Transmission, Wav, SENTENCE_GAP_S};

const MODEL_DIR: &str = "data/voice";
/// Longest part sent to the engine in one go; longer sentences are split on words.
const MAX_PART_CHARS: usize = 400;
/// Sentences shorter than this ("Good lap.") ride along with the next one rather than costing
/// a request of their own.
const MIN_PART_CHARS: usize = 40;
const RUNNER_HINT: &str = "Install uv (https://docs.astral.sh/uv/) so the neural voice can run.";
/// espeak voice used when it's only the fallback for a neural engine.
const FALLBACK_ESPEAK_VOICE: &str = "en-gb";

/// Which voice engine to use, picked with `--tts`. Falls back to espeak when it isn't set up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Tts {
    Piper,
    Kokoro,
    Espeak,
}

impl Tts {
    /// British female by default for both neural engines.
    pub fn default_voice(self) -> &'static str {
        match self {
            Tts::Piper => "en_GB-cori-high",
            Tts::Kokoro => "bf_emma",
            Tts::Espeak => FALLBACK_ESPEAK_VOICE,
        }
    }

    /// The name the UI shows and checks.
    fn name(self) -> &'static str {
        match self {
            Tts::Piper => "piper",
            Tts::Kokoro => "kokoro",
            Tts::Espeak => "espeak",
        }
    }
}

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Serialize)]
pub struct VoiceStatus {
    /// "piper", "kokoro", "espeak", or "none" when nothing can speak.
    pub engine: &'static str,
    pub voice: String,
    pub hint: Option<String>,
}

struct Sidecar {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl Drop for Sidecar {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// What this machine has, looked up once: PATH walks and model-file checks aren't free.
struct Detected {
    models_present: bool,
    /// Program and arguments that start the sidecar (`uv run …` or plain Python).
    runner: Option<(String, Vec<String>)>,
    espeak: Option<String>,
}

pub struct VoiceEngine {
    tts: Tts,
    voice: String,
    detected: OnceLock<Detected>,
    sidecar: Mutex<Option<Sidecar>>,
    /// Set when the sidecar could not start at all, so we stop retrying and use espeak.
    sidecar_error: Mutex<Option<String>>,
}

/// Picks the best available engine from what was detected, or `None` if nothing can speak.
/// Pure so it can be unit-tested.
fn select_engine(tts: Tts, neural_ready: bool, espeak_present: bool) -> Option<Tts> {
    if tts != Tts::Espeak && neural_ready {
        Some(tts)
    } else if espeak_present {
        Some(Tts::Espeak)
    } else {
        None
    }
}

impl VoiceEngine {
    /// `voice` is the engine's voice name; `None` uses the engine's default.
    pub fn new(tts: Tts, voice: Option<&str>) -> Self {
        Self {
            tts,
            voice: voice.unwrap_or(tts.default_voice()).to_string(),
            detected: OnceLock::new(),
            sidecar: Mutex::new(None),
            sidecar_error: Mutex::new(None),
        }
    }

    /// The sidecar script, its arguments, and the model files it needs.
    fn neural(&self) -> (&'static str, Vec<String>, Vec<PathBuf>) {
        let dir = Path::new(MODEL_DIR);
        match self.tts {
            Tts::Kokoro => (
                "voice/kokoro_tts.py",
                vec!["--models".into(), MODEL_DIR.into()],
                ["kokoro-v1.0.onnx", "voices-v1.0.bin"].iter().map(|f| dir.join(f)).collect(),
            ),
            _ => {
                let model = dir.join("piper").join(format!("{}.onnx", self.voice));
                let config = dir.join("piper").join(format!("{}.onnx.json", self.voice));
                ("voice/piper_tts.py", vec!["--model".into(), model.display().to_string()], vec![model, config])
            }
        }
    }

    fn setup_hint(&self) -> String {
        let (engine, arg) = match self.tts {
            Tts::Kokoro => ("Kokoro", "kokoro".to_string()),
            _ => ("Piper", format!("piper {}", self.voice)),
        };
        format!(
            "Run scripts/setup-voice.sh {arg} (macOS/Linux) or scripts/setup-voice.ps1 {arg} (Windows) to download the {engine} voice model."
        )
    }

    fn detected(&self) -> &Detected {
        self.detected.get_or_init(|| {
            let (script, args, files) = self.neural();
            let script_args = |extra: &[&str]| -> Vec<String> {
                extra.iter().map(|s| s.to_string()).chain(std::iter::once(script.to_string())).chain(args.iter().cloned()).collect()
            };
            let runner = match find_executable("uv") {
                Some(uv) => Some((uv, script_args(&["run", "--quiet"]))),
                // Without uv the dependencies must already be installed in this interpreter.
                None => find_executable("python3").or_else(|| find_executable("python")).map(|python| (python, script_args(&[]))),
            };
            Detected {
                models_present: files.iter().all(|f| f.is_file()) && Path::new(script).is_file(),
                runner,
                espeak: find_executable("espeak-ng").or_else(|| find_executable("espeak")),
            }
        })
    }

    fn sidecar_error(&self) -> Option<String> {
        self.sidecar_error.lock().ok().and_then(|e| e.clone())
    }

    fn set_sidecar_error(&self, err: &anyhow::Error) {
        if let Ok(mut slot) = self.sidecar_error.lock() {
            *slot = Some(format!("{err:#}"));
        }
    }

    /// The engine that will speak right now, or `None`.
    fn active(&self) -> Option<Tts> {
        let d = self.detected();
        select_engine(self.tts, d.models_present && d.runner.is_some() && self.sidecar_error().is_none(), d.espeak.is_some())
    }

    pub fn is_available(&self) -> bool {
        self.active().is_some()
    }

    pub fn status(&self) -> VoiceStatus {
        let active = self.active();
        let d = self.detected();
        let hint = if active == Some(self.tts) {
            None
        } else if self.tts == Tts::Espeak {
            Some("Install espeak-ng so the fallback voice can run.".to_string())
        } else if !d.models_present {
            Some(self.setup_hint())
        } else if d.runner.is_none() {
            Some(RUNNER_HINT.to_string())
        } else {
            self.sidecar_error().map(|err| format!("The {:?} voice failed to start: {err}", self.tts))
        };
        VoiceStatus { engine: active.map_or("none", Tts::name), voice: self.voice.clone(), hint }
    }

    /// Starts the neural sidecar ahead of the first request, so the model load (a few seconds)
    /// doesn't land on the first thing the coach says.
    pub fn warm_up(&self) {
        if !matches!(self.active(), Some(Tts::Piper | Tts::Kokoro)) {
            return;
        }
        let Ok(mut guard) = self.sidecar.lock() else { return };
        if guard.is_some() {
            return;
        }
        match self.start_sidecar() {
            Ok(sidecar) => *guard = Some(sidecar),
            Err(err) => self.set_sidecar_error(&err),
        }
    }

    /// The message in spoken form, split into sentence-sized parts for playing one by one, so
    /// the driver hears the first sentence while the rest is still being generated.
    pub fn plan(&self, text: &str) -> Vec<String> {
        plan_parts(&crate::voice::speech::for_speech(text), MIN_PART_CHARS, MAX_PART_CHARS)
    }

    /// One part from `plan` (already in spoken form). Parts played back to back in order make
    /// one continuous transmission.
    pub fn speak_part(&self, spoken: &str, radio: bool, part: Transmission) -> Result<Vec<u8>> {
        let mut wav = self.synth(spoken)?;
        if radio {
            wav = radio_filter(&wav, part);
        } else if !part.last {
            let gap = (wav.sample_rate as f32 * SENTENCE_GAP_S) as usize;
            wav.samples.extend(std::iter::repeat_n(0.0f32, gap));
        }
        Ok(write_wav(&wav))
    }

    /// Synthesises with the chosen engine, falling back to espeak if the neural engine fails.
    fn synth(&self, text: &str) -> Result<Wav> {
        if text.trim().is_empty() {
            bail!("Nothing to say.");
        }
        match self.active() {
            None => bail!("No voice engine available. {}", self.setup_hint()),
            Some(Tts::Espeak) => self.synth_espeak(text),
            Some(engine) => self.synth_neural(text).or_else(|err| {
                eprintln!("{engine:?} synthesis failed: {err:#}");
                if self.detected().espeak.is_none() {
                    return Err(err);
                }
                self.synth_espeak(text)
            }),
        }
    }

    fn synth_espeak(&self, text: &str) -> Result<Wav> {
        let path = self.detected().espeak.as_ref().ok_or_else(|| anyhow!("espeak is not installed"))?;
        // The user's choice of voice applies when espeak is what they asked for, not when it's a fallback.
        let voice = if self.tts == Tts::Espeak { self.voice.as_str() } else { FALLBACK_ESPEAK_VOICE };
        let output = Command::new(path)
            .args(["-v", voice, "--stdout"])
            .arg(text)
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .context("failed to run espeak")?;
        if !output.status.success() {
            bail!("espeak returned a non-zero exit code");
        }
        read_wav(&output.stdout)
    }

    fn start_sidecar(&self) -> Result<Sidecar> {
        let (program, args) = self.detected().runner.as_ref().ok_or_else(|| anyhow!(RUNNER_HINT))?;
        let mut child = Command::new(program)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .context("failed to start the voice sidecar")?;
        let stdin = child.stdin.take().context("sidecar stdin unavailable")?;
        let stdout = child.stdout.take().context("sidecar stdout unavailable")?;
        let mut sidecar = Sidecar { child, stdin, stdout: BufReader::new(stdout) };

        let mut line = String::new();
        if sidecar.stdout.read_line(&mut line)? == 0 {
            bail!("voice sidecar exited before becoming ready");
        }
        let reply: serde_json::Value = serde_json::from_str(line.trim()).context("bad ready message from sidecar")?;
        if reply["ready"].as_bool() != Some(true) {
            bail!("{}", reply["error"].as_str().unwrap_or("voice model failed to load"));
        }
        Ok(sidecar)
    }

    fn synth_neural(&self, text: &str) -> Result<Wav> {
        let id = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("pitcrew-tts-{}-{id}.wav", std::process::id()));
        let request = serde_json::json!({
            "text": text,
            "voice": self.voice,
            "out": path.display().to_string(),
        });

        let result = (|| -> Result<Wav> {
            let mut guard = self.sidecar.lock().map_err(|_| anyhow!("voice engine lock poisoned"))?;
            let mut last_err = anyhow!("sidecar unavailable");
            // Two tries: the second one restarts a sidecar that died.
            for _ in 0..2 {
                if guard.is_none() {
                    match self.start_sidecar() {
                        Ok(sidecar) => *guard = Some(sidecar),
                        Err(err) => {
                            self.set_sidecar_error(&err);
                            return Err(err);
                        }
                    }
                }
                match roundtrip(guard.as_mut().expect("sidecar started"), &request) {
                    Ok(()) => {
                        let bytes = std::fs::read(&path).context("sidecar did not write audio")?;
                        return read_wav(&bytes);
                    }
                    Err(RoundtripError::Synthesis(err)) => return Err(err),
                    Err(RoundtripError::Io(err)) => {
                        *guard = None;
                        last_err = err;
                    }
                }
            }
            Err(last_err)
        })();
        let _ = std::fs::remove_file(&path);
        result
    }
}

enum RoundtripError {
    /// The sidecar is alive but refused/failed this request.
    Synthesis(anyhow::Error),
    /// The pipe broke: the sidecar probably died.
    Io(anyhow::Error),
}

fn roundtrip(sidecar: &mut Sidecar, request: &serde_json::Value) -> std::result::Result<(), RoundtripError> {
    let io = |err: std::io::Error| RoundtripError::Io(err.into());
    writeln!(sidecar.stdin, "{request}").map_err(io)?;
    sidecar.stdin.flush().map_err(io)?;
    let mut line = String::new();
    let read = sidecar.stdout.read_line(&mut line).map_err(io)?;
    if read == 0 {
        return Err(RoundtripError::Io(anyhow!("voice sidecar closed its output")));
    }
    let reply: serde_json::Value =
        serde_json::from_str(line.trim()).map_err(|e| RoundtripError::Io(anyhow!("bad sidecar reply: {e}")))?;
    if reply["ok"].as_bool() == Some(true) {
        Ok(())
    } else {
        Err(RoundtripError::Synthesis(anyhow!(
            "{}",
            reply["error"].as_str().unwrap_or("synthesis failed")
        )))
    }
}

/// Sentence-sized parts: one sentence each, short ones merged into the next, long ones split.
fn plan_parts(text: &str, min: usize, max: usize) -> Vec<String> {
    let mut parts: Vec<String> = Vec::new();
    let mut pending = String::new();
    for paragraph in text.split("\n\n") {
        let paragraph = paragraph.split_whitespace().collect::<Vec<_>>().join(" ");
        for sentence in split_sentences(&paragraph) {
            for piece in split_long(&sentence, max) {
                if !pending.is_empty() {
                    pending.push(' ');
                }
                pending.push_str(&piece);
                if pending.chars().count() >= min {
                    parts.push(std::mem::take(&mut pending));
                }
            }
        }
    }
    if !pending.is_empty() {
        match parts.last_mut() {
            Some(last) if last.chars().count() + 1 + pending.chars().count() <= max => {
                last.push(' ');
                last.push_str(&pending);
            }
            _ => parts.push(pending),
        }
    }
    parts
}

fn split_sentences(text: &str) -> Vec<String> {
    let mut sentences = Vec::new();
    let mut current = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        current.push(c);
        if matches!(c, '.' | '!' | '?') && chars.peek().is_none_or(|n| n.is_whitespace()) {
            sentences.push(current.trim().to_string());
            current.clear();
        }
    }
    if !current.trim().is_empty() {
        sentences.push(current.trim().to_string());
    }
    sentences
}

fn split_long(sentence: &str, max: usize) -> Vec<String> {
    if sentence.chars().count() <= max {
        return vec![sentence.to_string()];
    }
    let mut pieces = Vec::new();
    let mut current = String::new();
    for word in sentence.split_whitespace() {
        if !current.is_empty() && current.chars().count() + 1 + word.chars().count() > max {
            pieces.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(word);
    }
    if !current.is_empty() {
        pieces.push(current);
    }
    pieces
}

fn find_executable(command: &str) -> Option<String> {
    let command_path = std::path::Path::new(command);
    if command_path.is_file() {
        return Some(command.to_string());
    }

    let path_var = std::env::var("PATH").unwrap_or_default();

    #[cfg(windows)]
    let windows_exts: Vec<String> = {
        let pathext = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_string());
        pathext
            .split(';')
            .filter(|ext| !ext.trim().is_empty())
            .map(|ext| ext.trim().to_ascii_lowercase())
            .collect()
    };

    #[cfg(windows)]
    let has_extension = command_path.extension().is_some();

    for entry in std::env::split_paths(&path_var) {
        let candidate = entry.join(command);
        if candidate.is_file() {
            return Some(candidate.to_string_lossy().to_string());
        }

        #[cfg(windows)]
        if !has_extension {
            for ext in &windows_exts {
                let with_ext = entry.join(format!("{command}{ext}"));
                if with_ext.is_file() {
                    return Some(with_ext.to_string_lossy().to_string());
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_gives_one_sentence_per_part_and_merges_short_ones() {
        let text = "Good lap. You're losing about three tenths into Turn five, so brake a touch later. \
                    Then carry more speed to the apex and get on the power early. Nice.";
        let parts = plan_parts(text, 40, 400);
        assert_eq!(
            parts,
            vec![
                "Good lap. You're losing about three tenths into Turn five, so brake a touch later.",
                "Then carry more speed to the apex and get on the power early. Nice.",
            ]
        );
        assert_eq!(plan_parts("Box.", 40, 400), vec!["Box."]);
        assert!(plan_parts("  ", 40, 400).is_empty());
    }

    #[test]
    fn engine_selection() {
        assert_eq!(select_engine(Tts::Piper, true, true), Some(Tts::Piper));
        assert_eq!(select_engine(Tts::Kokoro, true, false), Some(Tts::Kokoro));
        assert_eq!(select_engine(Tts::Piper, false, true), Some(Tts::Espeak));
        assert_eq!(select_engine(Tts::Espeak, true, true), Some(Tts::Espeak));
        assert_eq!(select_engine(Tts::Piper, false, false), None);
        assert_eq!(select_engine(Tts::Espeak, true, false), None);
    }

    #[test]
    fn piper_model_paths_follow_the_voice_name() {
        let engine = VoiceEngine::new(Tts::Piper, None);
        let (script, args, files) = engine.neural();
        assert_eq!(script, "voice/piper_tts.py");
        assert!(args[1].ends_with("en_GB-cori-high.onnx"));
        assert!(files[1].ends_with("piper/en_GB-cori-high.onnx.json"));
        assert_eq!(VoiceEngine::new(Tts::Kokoro, None).voice, "bf_emma");
    }

    #[test]
    fn plan_splits_overlong_sentences_on_words() {
        let text = "word ".repeat(200);
        let parts = plan_parts(&text, 40, 50);
        assert!(parts.len() > 1);
        assert!(parts.iter().all(|p| p.chars().count() <= 50));
        assert_eq!(parts.join(" ").split_whitespace().count(), 200);
    }
}
