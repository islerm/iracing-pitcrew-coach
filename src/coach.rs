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
    let ollama_path = find_executable("ollama");
    let Some(ollama_path) = ollama_path else {
        return "Ollama is not installed or not on PATH. Fallback analysis: focus on the sector where you lose the most time, and smooth your braking and throttle there.".to_string();
    };

    let prompt = build_prompt(summary);
    match Command::new(ollama_path)
        .arg("run")
        .arg("--nowordwrap")
        .arg(model_name)
        .arg(prompt)
        .output()
    {
        Ok(result) if result.status.success() => {
            let stdout = strip_ansi(&String::from_utf8_lossy(&result.stdout))
                .trim()
                .trim_matches('"')
                .trim()
                .to_string();
            if !stdout.is_empty() {
                stdout
            } else {
                summary.suggestions.join("; ")
            }
        }
        Ok(_) => summary.suggestions.join("; "),
        Err(_) => summary.suggestions.join("; "),
    }
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
