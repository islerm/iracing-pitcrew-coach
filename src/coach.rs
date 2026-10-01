use anyhow::{Context, Result};
use std::process::Command;

use crate::analysis::build_prompt;
use crate::types::SessionSummary;

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

pub fn generate_feedback(summary: &SessionSummary, model_name: &str) -> String {
    match ask_model(&build_prompt(summary), model_name) {
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

/// Runs one prompt through the local Ollama model and returns the cleaned-up answer.
pub fn ask_model(prompt: &str, model_name: &str) -> Result<String, ModelError> {
    let ollama_path = find_executable("ollama").ok_or(ModelError::NotInstalled)?;
    let run = |think_flag: bool| {
        let mut command = Command::new(&ollama_path);
        command.arg("run").arg("--nowordwrap");
        // Reasoning models (Qwen 3.5, Gemma 4, …) would otherwise think out loud first: slower,
        // and the reasoning would land in the answer. Other models ignore the flag.
        if think_flag {
            command.arg("--think=false");
        }
        command.arg(model_name).arg(prompt).output().map_err(|err| ModelError::Failed(err.to_string()))
    };
    let mut result = run(true)?;
    // Ollama releases before thinking support don't know the flag.
    if !result.status.success() && String::from_utf8_lossy(&result.stderr).contains("unknown flag") {
        result = run(false)?;
    }
    if !result.status.success() {
        let stderr = strip_ansi(&String::from_utf8_lossy(&result.stderr)).trim().to_string();
        return Err(ModelError::Failed(if stderr.is_empty() { format!("exit status {}", result.status) } else { stderr }));
    }
    let stdout = strip_ansi(&String::from_utf8_lossy(&result.stdout))
        .trim()
        .trim_matches('"')
        .trim()
        .to_string();
    if stdout.is_empty() {
        Err(ModelError::Failed("empty answer".to_string()))
    } else {
        Ok(stdout)
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
    let ollama_path = find_executable("ollama").ok_or(ModelError::NotInstalled)?;
    let result = Command::new(ollama_path).arg("list").output().map_err(|err| ModelError::Failed(err.to_string()))?;
    if !result.status.success() {
        let stderr = strip_ansi(&String::from_utf8_lossy(&result.stderr)).trim().to_string();
        return Err(ModelError::Failed(if stderr.is_empty() { format!("exit status {}", result.status) } else { stderr }));
    }
    Ok(parse_model_list(&strip_ansi(&String::from_utf8_lossy(&result.stdout))))
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

pub fn speak_feedback(text: &str, voice: &str) -> Result<()> {
    let espeak_path = find_executable("espeak-ng").or_else(|| find_executable("espeak"));
    let Some(espeak_path) = espeak_path else {
        println!("Speech synthesis is not installed, so the feedback is only printed to the terminal.");
        return Ok(());
    };

    let status = Command::new(espeak_path)
        .arg("-v")
        .arg(voice)
        .arg(text)
        .status()
        .with_context(|| "failed to invoke speech synthesis")?;

    if !status.success() {
        anyhow::bail!("espeak returned a non-zero exit code");
    }

    Ok(())
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
