use std::io::{ErrorKind, Write};
use std::process::{Command, Output, Stdio};

pub mod prompt;
pub mod radio_call;

use crate::analysis::SessionSummary;
use crate::coach::prompt::run_prompt;

pub fn generate_feedback(summary: &SessionSummary, model_name: &str) -> String {
    match ask_model(&run_prompt(summary), model_name) {
        Ok(text) => text,
        Err(ModelError::NotInstalled) => "Ollama is not installed or not on PATH. Fallback analysis: focus on the sector where you lose the most time, and smooth your braking and throttle there.".to_string(),
        Err(ModelError::Failed(_)) => summary.suggestions.join("; "),
    }
}

#[derive(Debug)]
pub enum ModelError {
    NotInstalled,
    Failed(String),
}

impl std::fmt::Display for ModelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ModelError::NotInstalled => write!(f, "Ollama is not installed or not on PATH."),
            ModelError::Failed(reason) => write!(f, "The coach model didn't answer: {reason}"),
        }
    }
}

/// Runs `ollama` with `args`, feeding it `stdin` (the prompt can be longer than a command line
/// allows on Windows). A missing executable means Ollama isn't installed.
fn ollama(args: &[&str], stdin: Option<&str>) -> Result<Output, ModelError> {
    let failed = |err: std::io::Error| ModelError::Failed(err.to_string());
    let mut child = Command::new("ollama")
        .args(args)
        .stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| if err.kind() == ErrorKind::NotFound { ModelError::NotInstalled } else { failed(err) })?;
    if let (Some(text), Some(mut pipe)) = (stdin, child.stdin.take()) {
        pipe.write_all(text.as_bytes()).map_err(failed)?;
    }
    child.wait_with_output().map_err(failed)
}

/// The cleaned-up stdout of a finished `ollama` run, or why it failed.
fn output_text(result: &Output) -> Result<String, ModelError> {
    if !result.status.success() {
        let stderr = strip_ansi(&String::from_utf8_lossy(&result.stderr)).trim().to_string();
        return Err(ModelError::Failed(if stderr.is_empty() { format!("exit status {}", result.status) } else { stderr }));
    }
    Ok(strip_ansi(&String::from_utf8_lossy(&result.stdout)))
}

/// Where the Ollama server listens.
const OLLAMA_ADDR: &str = "127.0.0.1:11434";
/// How long Ollama keeps the model in memory after answering. Ollama's default is 5 minutes;
/// the coach is asked rarely, and a resident model holds GPU memory the sim could use.
const KEEP_ALIVE: &str = "1m";
/// Longest answer, in tokens: a few paragraphs.
const MAX_ANSWER_TOKENS: usize = 900;

/// Context window for a prompt: room for the prompt (~3.5 characters a token, rounded up)
/// and the answer. Ollama reserves memory for the whole window, and its default (4096 or
/// more) is bigger than the coach's prompts need; too small would cut the prompt off.
fn context_for(prompt: &str) -> usize {
    let needed = prompt.chars().count() * 2 / 7 + MAX_ANSWER_TOKENS + 256;
    needed.div_ceil(1024).clamp(2, 16) * 1024
}

enum HttpError {
    /// Nothing listening: try the `ollama` command, which also reports a missing install.
    Unreachable,
    Failed(String),
}

/// One prompt through Ollama's HTTP API, which (unlike `ollama run`) takes the context size
/// and how long to keep the model loaded. HTTP/1.0, so the reply is a plain body read to EOF.
fn generate_http(prompt: &str, model_name: &str) -> Result<String, HttpError> {
    use std::io::Read;
    use std::net::{SocketAddr, TcpStream};
    use std::time::Duration;

    let body = serde_json::json!({
        "model": model_name,
        "prompt": prompt,
        "stream": false,
        // Reasoning models (Qwen 3.5, Gemma 4, …) would otherwise think out loud first:
        // slower, and the reasoning would land in the answer.
        "think": false,
        "keep_alive": KEEP_ALIVE,
        "options": { "num_ctx": context_for(prompt), "num_predict": MAX_ANSWER_TOKENS },
    })
    .to_string();

    let addr: SocketAddr = OLLAMA_ADDR.parse().expect("valid address");
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(2)).map_err(|_| HttpError::Unreachable)?;
    let failed = |err: std::io::Error| HttpError::Failed(err.to_string());
    // Loading a big model and answering can take a while on a busy GPU.
    stream.set_read_timeout(Some(Duration::from_secs(600))).map_err(failed)?;
    let head = format!(
        "POST /api/generate HTTP/1.0\r\nHost: {OLLAMA_ADDR}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).and_then(|_| stream.write_all(body.as_bytes())).map_err(failed)?;
    let mut reply = Vec::new();
    stream.read_to_end(&mut reply).map_err(failed)?;

    let reply = String::from_utf8_lossy(&reply);
    let (head, json) = reply.split_once("\r\n\r\n").ok_or_else(|| HttpError::Failed("malformed reply from Ollama".into()))?;
    let ok = head.split_whitespace().nth(1) == Some("200");
    let value: serde_json::Value = serde_json::from_str(json).map_err(|err| HttpError::Failed(format!("unreadable reply from Ollama: {err}")))?;
    match (ok, value.get("response").and_then(|v| v.as_str()), value.get("error").and_then(|v| v.as_str())) {
        (true, Some(text), _) => Ok(text.to_string()),
        (_, _, Some(error)) => Err(HttpError::Failed(error.to_string())),
        _ => Err(HttpError::Failed(format!("Ollama answered: {}", head.lines().next().unwrap_or_default()))),
    }
}

/// The same through the `ollama` command, for when the server can't be reached directly
/// (the command starts it, and tells us when Ollama isn't installed).
fn generate_cli(prompt: &str, model_name: &str) -> Result<String, ModelError> {
    let mut result = ollama(&["run", "--nowordwrap", "--think=false", "--keepalive", KEEP_ALIVE, model_name], Some(prompt))?;
    // Ollama releases before thinking support don't know the flags.
    if !result.status.success() && String::from_utf8_lossy(&result.stderr).contains("unknown flag") {
        result = ollama(&["run", "--nowordwrap", model_name], Some(prompt))?;
    }
    output_text(&result)
}

/// Runs one prompt through the local Ollama model and returns the cleaned-up answer.
pub fn ask_model(prompt: &str, model_name: &str) -> Result<String, ModelError> {
    let raw = match generate_http(prompt, model_name) {
        Ok(text) => text,
        Err(HttpError::Failed(reason)) => return Err(ModelError::Failed(reason)),
        Err(HttpError::Unreachable) => generate_cli(prompt, model_name)?,
    };
    let answer = raw.trim().trim_matches('"').trim().to_string();
    if answer.is_empty() {
        Err(ModelError::Failed("empty answer".to_string()))
    } else {
        Ok(answer)
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct InstalledModel {
    pub name: String,
    /// As Ollama prints it, e.g. "4.7 GB".
    pub size: String,
}

/// Models already downloaded to this PC, from `ollama list`.
pub fn installed_models() -> Result<Vec<InstalledModel>, ModelError> {
    Ok(parse_model_list(&output_text(&ollama(&["list"], None)?)?))
}

/// Parses `ollama list`: a header, then `NAME  ID  SIZE  MODIFIED` rows, where SIZE is two
/// words ("4.7 GB") and MODIFIED several ("4 days ago").
fn parse_model_list(text: &str) -> Vec<InstalledModel> {
    text.lines()
        .skip(1)
        .filter_map(|line| {
            let cols: Vec<&str> = line.split_whitespace().collect();
            let name = cols.first()?.to_string();
            let size = match (cols.get(2), cols.get(3)) {
                (Some(n), Some(unit)) => format!("{n} {unit}"),
                _ => String::new(),
            };
            Some(InstalledModel { name, size })
        })
        .collect()
}

/// Removes terminal escape sequences (spinner, cursor moves) that Ollama writes to stdout.
fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                // CSI sequence: parameters until a final byte in '@'..='~'.
                while let Some(&next) = chars.peek() {
                    chars.next();
                    if ('@'..='~').contains(&next) {
                        break;
                    }
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ollama_list() {
        let text = "NAME               ID              SIZE      MODIFIED    \n\
                    qwen2.5:7b         845dbda0ea48    4.7 GB    4 days ago     \n\
                    llama3.2:latest    a80c4f17acd5    2.0 GB    11 days ago    \n";
        let models = parse_model_list(text);
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].name, "qwen2.5:7b");
        assert_eq!(models[1].size, "2.0 GB");
        assert!(parse_model_list("NAME ID SIZE MODIFIED\n").is_empty());
    }

    /// Needs Ollama running with llama3.2 pulled: `cargo test -- --ignored ollama_answers`.
    #[test]
    #[ignore]
    fn ollama_answers_over_http() {
        let answer = generate_http("Reply with the single word: ready", "llama3.2").map_err(|err| match err {
            HttpError::Unreachable => "Ollama isn't running".to_string(),
            HttpError::Failed(reason) => reason,
        });
        assert!(answer.expect("answer").to_lowercase().contains("ready"));
    }

    #[test]
    fn context_fits_the_prompt_and_answer() {
        assert_eq!(context_for("short"), 2048);
        // ~3000 tokens of prompt plus the answer.
        let long = "x".repeat(10_500);
        let ctx = context_for(&long);
        assert!(ctx >= 3000 + MAX_ANSWER_TOKENS && ctx.is_multiple_of(1024), "{ctx}");
        assert_eq!(context_for(&"x".repeat(1_000_000)), 16 * 1024);
    }
}
