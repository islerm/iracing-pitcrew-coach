use std::io::{ErrorKind, Write};
use std::process::{Command, Output, Stdio};

pub mod prompt;

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

/// Runs one prompt through the local Ollama model and returns the cleaned-up answer.
pub fn ask_model(prompt: &str, model_name: &str) -> Result<String, ModelError> {
    // Reasoning models (Qwen 3.5, Gemma 4, …) would otherwise think out loud first: slower,
    // and the reasoning would land in the answer. Other models ignore the flag.
    let mut result = ollama(&["run", "--nowordwrap", "--think=false", model_name], Some(prompt))?;
    // Ollama releases before thinking support don't know the flag.
    if !result.status.success() && String::from_utf8_lossy(&result.stderr).contains("unknown flag") {
        result = ollama(&["run", "--nowordwrap", model_name], Some(prompt))?;
    }
    let answer = output_text(&result)?.trim().trim_matches('"').trim().to_string();
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
}
