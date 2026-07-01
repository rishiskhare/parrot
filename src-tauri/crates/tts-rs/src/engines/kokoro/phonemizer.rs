use std::borrow::Cow;
use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use super::model::KokoroError;

/// Configuration for locating the espeak-ng binary and its data directory.
///
/// When paths are `None`, falls back to `"espeak-ng"` on PATH with its
/// compiled-in default data directory.
#[derive(Debug, Clone, Default)]
pub struct EspeakConfig {
    /// Path to the espeak-ng binary. Falls back to `"espeak-ng"` on PATH.
    pub bin_path: Option<PathBuf>,
    /// Path to the espeak-ng-data directory. When set, passed via `--path`.
    pub data_path: Option<PathBuf>,
}

/// Map a voice name prefix to an espeak-ng language code.
///
/// Voice names follow the pattern `{prefix}_{name}` where the two-character
/// prefix encodes the language.
pub fn voice_lang(voice: &str) -> &'static str {
    let prefix = &voice[..voice.len().min(2)];
    match prefix {
        "af" | "am" => "en-us",
        "bf" | "bm" => "en-gb",
        "ef" | "em" => "es",
        "ff" => "fr",
        "hf" | "hm" => "hi",
        "if" | "im" => "it",
        "jf" | "jm" => "ja",
        "pf" | "pm" => "pt-br",
        "zf" | "zm" => "cmn",
        _ => "en-us",
    }
}

/// Convert text to Kokoro phoneme token IDs via espeak-ng.
///
/// # Arguments
/// - `text`: The input text to phonemize
/// - `lang`: espeak-ng language code (e.g. `"en-us"`, `"fr"`, `"ja"`, `"cmn"`)
/// - `vocab`: Mapping from IPA characters to token IDs
///
/// # Returns
/// A `Vec<i64>` of token IDs. Characters not in the vocab are silently dropped,
/// matching the behavior of the Python reference implementation.
pub fn phonemize(
    text: &str,
    lang: &str,
    vocab: &HashMap<char, i64>,
    espeak: &EspeakConfig,
) -> Result<Vec<i64>, KokoroError> {
    let parts = split_text_parts(text);
    if parts.is_empty() {
        return Ok(Vec::new());
    }

    let text_segments: Vec<&str> = parts
        .iter()
        .filter_map(|part| match part {
            TextPart::Text(segment) => Some(segment.as_str()),
            TextPart::Punct(_) => None,
        })
        .collect();

    let segment_ids = if text_segments.is_empty() {
        Vec::new()
    } else {
        phonemize_segments_batch(&text_segments, lang, vocab, espeak)?
    };

    let mut ids = Vec::new();
    let mut segment_index = 0usize;
    for part in parts {
        match part {
            TextPart::Text(_) => {
                if let Some(chunk) = segment_ids.get(segment_index) {
                    ids.extend_from_slice(chunk);
                }
                segment_index += 1;
            }
            TextPart::Punct(ch) => {
                if let Some(&id) = vocab.get(&ch) {
                    ids.push(id);
                }
            }
        }
    }

    Ok(ids)
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum TextPart {
    Text(String),
    Punct(char),
}

fn split_text_parts(text: &str) -> Vec<TextPart> {
    let mut parts = Vec::new();
    let mut current = String::new();

    for (idx, ch) in text.char_indices() {
        let ch_len = ch.len_utf8();
        if let Some(punct) = map_boundary_punctuation(ch) {
            if !is_numeric_connector_between_digits(text, idx, ch_len, ch) {
                flush_text_part(&mut parts, &mut current);
                parts.push(TextPart::Punct(punct));
                continue;
            }
        }

        if ch.is_whitespace() {
            if !current.is_empty() && !current.ends_with(' ') {
                current.push(' ');
            }
            continue;
        }

        current.push(ch);
    }

    flush_text_part(&mut parts, &mut current);
    parts
}

fn flush_text_part(parts: &mut Vec<TextPart>, current: &mut String) {
    let trimmed = current.trim();
    if trimmed.is_empty() {
        current.clear();
        return;
    }
    parts.push(TextPart::Text(trimmed.to_string()));
    current.clear();
}

fn map_boundary_punctuation(ch: char) -> Option<char> {
    match ch {
        '.' | '!' | '?' | ',' | ';' | ':' | '—' | '…' | '"' | '(' | ')' | '\u{201c}'
        | '\u{201d}' => Some(ch),
        '\n' | '\r' => Some('.'),
        _ => None,
    }
}

fn is_numeric_connector_between_digits(text: &str, idx: usize, ch_len: usize, ch: char) -> bool {
    if !matches!(ch, '.' | ',') {
        return false;
    }

    let prev = text[..idx].chars().next_back();
    let next = text[idx + ch_len..].chars().next();

    matches!(
        (prev, next),
        (Some(left), Some(right)) if left.is_ascii_digit() && right.is_ascii_digit()
    )
}

fn phonemize_segments_batch(
    segments: &[&str],
    lang: &str,
    vocab: &HashMap<char, i64>,
    espeak: &EspeakConfig,
) -> Result<Vec<Vec<i64>>, KokoroError> {
    let batched_input = segments.join("\n");
    let output = run_espeak(&batched_input, lang, espeak)?;
    let lines: Vec<&str> = output.lines().collect();

    // espeak-ng should emit one line per input line for stdin mode.
    // If this assumption breaks, fall back to per-segment invocation.
    if lines.len() != segments.len() {
        return segments
            .iter()
            .map(|segment| {
                let output = run_espeak(segment, lang, espeak)?;
                Ok(ipa_to_ids(&output, vocab))
            })
            .collect();
    }

    Ok(lines.iter().map(|line| ipa_to_ids(line, vocab)).collect())
}

fn run_espeak(input: &str, lang: &str, espeak: &EspeakConfig) -> Result<String, KokoroError> {
    let bin = espeak
        .bin_path
        .as_deref()
        .map(|p| p.as_os_str().to_owned())
        .unwrap_or_else(|| std::ffi::OsString::from("espeak-ng"));
    let mut cmd = Command::new(&bin);
    cmd.args(["--ipa", "--stdin", "-q", "-v", lang]);
    if let Some(data_path) = espeak.data_path.as_deref() {
        cmd.arg("--path").arg(data_path);
    }
    // When using a bundled binary, shared libraries (libespeak-ng.so,
    // libpcaudio.so) are placed next to the binary.  On Linux, the dynamic
    // linker needs LD_LIBRARY_PATH to find them (RPATH may not be set).
    #[cfg(target_os = "linux")]
    if let Some(bin_dir) = espeak.bin_path.as_deref().and_then(|p| p.parent()) {
        cmd.env("LD_LIBRARY_PATH", bin_dir);
    }
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                KokoroError::EspeakNotFound
            } else {
                KokoroError::Io(e)
            }
        })?;

    if let Some(mut stdin) = child.stdin.take() {
        // espeak-ng treats stdin as line-oriented input. Without a final line terminator,
        // the last token can be under-processed. Enforce a canonical, newline-terminated
        // payload as part of this I/O contract.
        let stdin_payload = canonicalize_espeak_stdin_payload(input);
        stdin
            .write_all(stdin_payload.as_bytes())
            .map_err(KokoroError::Io)?;
    }

    let output = child.wait_with_output().map_err(KokoroError::Io)?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(KokoroError::PhonemizerFailed(format!(
            "espeak-ng exited with code {:?}: {stderr}",
            output.status.code()
        )));
    }

    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn canonicalize_espeak_stdin_payload(input: &str) -> Cow<'_, str> {
    if input.ends_with('\n') {
        Cow::Borrowed(input)
    } else {
        Cow::Owned(format!("{input}\n"))
    }
}

fn ipa_to_ids(ipa: &str, vocab: &HashMap<char, i64>) -> Vec<i64> {
    let mut ids = Vec::new();
    for line in ipa.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        for ch in line.chars() {
            if ch == '_' {
                continue;
            }
            if let Some(&id) = vocab.get(&ch) {
                ids.push(id);
            }
        }
    }
    ids
}

#[cfg(test)]
mod tests {
    use super::{
        canonicalize_espeak_stdin_payload, phonemize, run_espeak, split_text_parts, EspeakConfig,
        TextPart,
    };
    use crate::engines::kokoro::vocab::hardcoded_vocab;
    use std::process::Command;

    fn espeak_available() -> bool {
        Command::new("espeak-ng")
            .arg("--version")
            .output()
            .is_ok()
    }

    #[test]
    fn splits_text_and_punctuation_parts() {
        let parts = split_text_parts("Hello, world. Testing!");
        assert_eq!(
            parts,
            vec![
                TextPart::Text("Hello".to_string()),
                TextPart::Punct(','),
                TextPart::Text("world".to_string()),
                TextPart::Punct('.'),
                TextPart::Text("Testing".to_string()),
                TextPart::Punct('!'),
            ]
        );
    }

    #[test]
    fn keeps_decimal_and_thousands_separators_inside_text() {
        let parts = split_text_parts("Version 2.0 reached 1,000 users.");
        assert_eq!(
            parts,
            vec![
                TextPart::Text("Version 2.0 reached 1,000 users".to_string()),
                TextPart::Punct('.'),
            ]
        );
    }

    #[test]
    fn still_splits_comma_when_not_between_digits() {
        let parts = split_text_parts("Value 2, next");
        assert_eq!(
            parts,
            vec![
                TextPart::Text("Value 2".to_string()),
                TextPart::Punct(','),
                TextPart::Text("next".to_string()),
            ]
        );
    }

    #[test]
    fn appends_trailing_newline_for_espeak_stdin() {
        assert_eq!(canonicalize_espeak_stdin_payload("America"), "America\n");
    }

    #[test]
    fn keeps_single_trailing_newline_for_espeak_stdin() {
        assert_eq!(canonicalize_espeak_stdin_payload("America\n"), "America\n");
    }

    #[test]
    fn espeak_output_is_stable_with_or_without_trailing_newline() {
        if !espeak_available() {
            return;
        }

        let cfg = EspeakConfig::default();
        let without_newline =
            run_espeak("America", "en-us", &cfg).expect("espeak should succeed");
        let with_newline =
            run_espeak("America\n", "en-us", &cfg).expect("espeak should succeed");
        assert_eq!(
            without_newline.trim(),
            with_newline.trim(),
            "stdin canonicalization must prevent final-token truncation"
        );
    }

    #[test]
    fn phonemize_keeps_terminal_schwa_for_america() {
        if !espeak_available() {
            return;
        }

        let vocab = hardcoded_vocab();
        let cfg = EspeakConfig::default();
        let ids =
            phonemize("America", "en-us", &vocab, &cfg).expect("phonemize should succeed");
        let schwa_id = *vocab
            .get(&'ə')
            .expect("hardcoded vocab should include schwa");
        assert_eq!(
            ids.last(),
            Some(&schwa_id),
            "terminal schwa should be preserved for 'America'"
        );
    }
}
