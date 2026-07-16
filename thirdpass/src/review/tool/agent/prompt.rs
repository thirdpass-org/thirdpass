use anyhow::{format_err, Result};
use serde::Deserialize;
use serde_json::Value;
use std::path::PathBuf;
use std::str::FromStr;

use crate::review::comment::common::Position;
use crate::review::comment::{Comment, Selection};
use crate::review::common::{Priority, ReviewConfidence};

pub(super) const OUTPUT_SCHEMA: &str = r#"{
  "type": "object",
  "additionalProperties": false,
  "required": ["model", "summary", "confidence", "comments"],
  "properties": {
    "model": { "type": "string" },
    "summary": { "type": "string" },
    "confidence": { "type": "string", "enum": ["high", "medium", "low"] },
    "comments": {
      "type": "array",
      "items": {
        "type": "object",
        "additionalProperties": false,
        "required": ["comment", "security", "complexity", "file", "selection"],
        "properties": {
          "comment": { "type": "string" },
          "security": { "type": "string", "enum": ["critical", "medium", "low"] },
          "complexity": { "type": "string", "enum": ["critical", "medium", "low"] },
          "file": { "type": "string" },
          "selection": {
            "type": ["object", "null"],
            "additionalProperties": false,
            "required": ["start", "end"],
            "properties": {
              "start": {
                "type": "object",
                "additionalProperties": false,
                "required": ["line", "character"],
                "properties": {
                  "line": { "type": "integer", "minimum": 1 },
                  "character": { "type": "integer", "minimum": 1 }
                }
              },
              "end": {
                "type": "object",
                "additionalProperties": false,
                "required": ["line", "character"],
                "properties": {
                  "line": { "type": "integer", "minimum": 1 },
                  "character": { "type": "integer", "minimum": 1 }
                }
              }
            }
          }
        }
      }
    }
  }
}"#;

#[derive(Debug, Deserialize)]
pub(super) struct AgentOutput {
    pub(super) model: String,
    #[serde(default)]
    summary: Option<String>,
    #[serde(default)]
    confidence: Option<ReviewConfidence>,
    comments: Vec<AgentComment>,
}

impl AgentOutput {
    pub(super) fn into_run_result(
        self,
        model: String,
        run_metrics: Option<thirdpass_core::schema::AgentRunMetrics>,
    ) -> super::AgentRunResult {
        let comments = self
            .comments
            .into_iter()
            .map(|comment| comment.into_comment())
            .collect();

        super::AgentRunResult {
            model,
            comments,
            summary: self.summary.and_then(|value| {
                let trimmed = value.trim().to_string();
                if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed)
                }
            }),
            confidence: self.confidence,
            run_metrics,
        }
    }
}

#[derive(Debug, Deserialize)]
struct AgentComment {
    comment: String,
    security: Priority,
    complexity: Priority,
    #[serde(rename = "file")]
    path: PathBuf,
    #[serde(default)]
    selection: Option<Selection>,
}

impl AgentComment {
    fn into_comment(self) -> Comment {
        Comment {
            id: 0,
            security: self.security,
            complexity: self.complexity,
            path: self.path,
            message: self.comment,
            selection: self.selection,
        }
    }
}

pub(super) fn build_prompt(display_path: &str) -> String {
    format!(
        r#"You are a malicious-code reviewer for open-source dependency archives.
Your goal is to detect evidence of supply-chain compromise or malicious behavior.
This is NOT a general vulnerability audit: avoid generic "unsafe pattern" findings unless they
are used to execute hidden/encoded/remote/untrusted payloads or are unsafe-by-default.

Review ONLY the target file at the path below. You are in read-only mode.
Inspect the target file from the current workspace before returning JSON.
You may inspect other files in the package if your tool supports it, but only report issues in the target file.
If the target file is binary, unreadable, or not meaningful as text, treat the review as a reachability review:
inspect package metadata, install scripts, wrappers, source files, and manifests for references that execute,
load, unpack, import, or otherwise pass control to the target file.

Focus areas (security):
- install-time execution (preinstall/postinstall), hidden subprocess execution
- credential/secret harvesting (env vars, .npmrc, .ssh, cloud metadata, tokens)
- data exfiltration (network calls, webhooks, DNS, pastebins)
- hidden downloads or dynamic code loading (remote fetch + eval/exec, require from URL)
- obfuscation/deobfuscation used to construct or execute payloads (base64, XOR, RC4)
- persistence or environment tampering (shell profiles, PATH, startup files)
- crypto-mining or unrelated system probing

Focus areas (complexity):
- heavy obfuscation or packing, control-flow flattening
- reflection/dynamic dispatch that hides behavior
- deliberately confusing parsing/decoding pipelines that mask intent

Rules:
- Output ONLY valid JSON, no markdown, no extra keys.
- Always include a brief summary and confidence, even if there are no comments.
- The summary must be specific to the target file: describe what it appears to contain or do,
  and mention the security-relevant behavior you checked.
- If comments is empty, the summary must positively state that no concrete malicious or
  supply-chain indicators were found and briefly name the checked categories that were absent,
  such as install hooks, network/exfiltration, credential access, dynamic code loading,
  obfuscation, or persistence.
- Do not use generic clean summaries like "looks fine" or "no issues found" without explaining
  what was reviewed.
- If there are no concrete malicious or supply-chain indicators, return an empty comments list.
- Comments must be specific and actionable, tied to the shown code, and include evidence:
  behavior + trigger + impact + why it is suspicious.
- Comments may mention other files only as context for behavior in the target file.
- Do not report a comment if the suspicious behavior is only present in another file.
- For binary or unreadable target files, only report when another package file uses the target as an opaque executable,
  loadable payload, unpacked artifact, or surprising runtime asset.
- Each comment's file field and non-null selection must point to the target file.
- Bundled/minified code is in scope, but only report when behavior is clearly malicious or suspicious-by-default.
- Do NOT flag common patterns (eval/new Function/dynamic require) unless tied to executing
  encoded/remote/untrusted input or concealing a payload.
- Do not flag clearly intentional, explicitly signposted risky capabilities when they are consistent
  with the package's apparent purpose.
- Do flag misleading, hidden, or insecure-by-default behavior, including security-sensitive actions that are implicit,
  surprising, or not opt-in.
- Prefer false negatives over low-confidence findings; if uncertain, return no comments.
- Use selection only when you can point to exact lines; otherwise set it to null.
- Line/character numbers are 1-based.
- Do not speculate about other files.

Return ONLY valid JSON with this schema. Do NOT include any preamble or code fences.
{{
  "model": "<model name used>",
  "summary": "<one or two sentence target-specific summary of what was reviewed and what was found or ruled out>",
  "confidence": "high|medium|low",
  "comments": [
    {{
      "comment": "string (what is the issue and why it matters)",
      "security": "critical|medium|low",
      "complexity": "critical|medium|low",
      "file": "{file_path}",
      "selection": {{
        "start": {{"line": <int>, "character": <int>}},
        "end": {{"line": <int>, "character": <int>}}
      }}
    }}
  ]
}}

If no issues are found, return an empty comments list.

Target file path (relative to current workspace): {file_path}
"#,
        file_path = display_path
    )
}

pub(super) fn parse_agent_output(raw: &str) -> Result<AgentOutput> {
    let trimmed = raw.trim();
    if let Ok(output) = serde_json::from_str::<AgentOutput>(trimmed) {
        return Ok(output);
    }

    let extracted = extract_json_payload(raw).unwrap_or_else(|| trimmed.to_string());
    if let Ok(output) = serde_json::from_str::<AgentOutput>(&extracted) {
        return Ok(output);
    }

    let value: Value = serde_json::from_str(&extracted).map_err(|err| {
        format_err!(
            "Failed to parse agent JSON output: {}. Output: {}",
            err,
            extracted
        )
    })?;
    parse_agent_value(value)
}

fn extract_json_payload(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if let Some(start) = trimmed.find("```json") {
        let rest = &trimmed[start + "```json".len()..];
        if let Some(end) = rest.find("```") {
            return Some(rest[..end].trim().to_string());
        }
    }
    if let Some(start) = trimmed.find("```") {
        let rest = &trimmed[start + "```".len()..];
        if let Some(end) = rest.find("```") {
            return Some(rest[..end].trim().to_string());
        }
    }
    let start = trimmed.find('{')?;
    let end = trimmed.rfind('}')?;
    if end <= start {
        return None;
    }
    Some(trimmed[start..=end].to_string())
}

fn parse_agent_value(value: Value) -> Result<AgentOutput> {
    let model = value
        .get("model")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();
    let summary = value
        .get("summary")
        .and_then(|v| v.as_str())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let confidence_value = value
        .get("confidence")
        .and_then(|v| v.as_str())
        .or_else(|| {
            value
                .get("overall_security_confidence")
                .and_then(|v| v.as_str())
        });
    let confidence = match confidence_value {
        Some(value) => ReviewConfidence::from_str(value).ok(),
        None => None,
    };
    let comments_value = value
        .get("comments")
        .ok_or(format_err!("Agent output missing comments array"))?;
    let comments_array = comments_value
        .as_array()
        .ok_or(format_err!("Agent comments is not an array"))?;

    let mut comments = Vec::new();
    for entry in comments_array {
        let comment = entry
            .get("comment")
            .and_then(|v| v.as_str())
            .or_else(|| entry.get("description").and_then(|v| v.as_str()))
            .unwrap_or("")
            .trim()
            .to_string();
        if comment.is_empty() {
            log::warn!("Skipping agent comment without description.");
            continue;
        }

        let path_value = entry
            .get("file")
            .and_then(|v| v.as_str())
            .or_else(|| entry.get("path").and_then(|v| v.as_str()));
        let path_value = match path_value {
            Some(path_value) => path_value,
            None => {
                log::warn!("Skipping agent comment without file/path.");
                continue;
            }
        };

        let security = parse_priority(
            entry.get("security").and_then(|v| v.as_str()),
            entry.get("severity").and_then(|v| v.as_str()),
            entry.get("security_finding").and_then(|v| v.as_bool()),
        );
        let complexity = parse_complexity(
            entry.get("complexity").and_then(|v| v.as_str()),
            entry.get("complexity_finding").and_then(|v| v.as_bool()),
        );

        let selection = parse_selection(entry);
        comments.push(AgentComment {
            comment,
            security,
            complexity,
            path: PathBuf::from(path_value),
            selection,
        });
    }

    Ok(AgentOutput {
        model,
        summary,
        confidence,
        comments,
    })
}

fn parse_priority(
    priority_value: Option<&str>,
    severity_value: Option<&str>,
    security_finding: Option<bool>,
) -> Priority {
    if let Some(value) = priority_value {
        if let Ok(priority) = Priority::from_str(value) {
            return priority;
        }
    }
    if let Some(value) = severity_value {
        let value = value.to_lowercase();
        return match value.as_str() {
            "critical" | "high" => Priority::Critical,
            "medium" | "moderate" => Priority::Medium,
            "low" | "info" => Priority::Low,
            _ => Priority::Medium,
        };
    }
    if let Some(true) = security_finding {
        return Priority::Medium;
    }
    Priority::Low
}

fn parse_complexity(priority_value: Option<&str>, complexity_finding: Option<bool>) -> Priority {
    if let Some(value) = priority_value {
        if let Ok(priority) = Priority::from_str(value) {
            return priority;
        }
    }
    if let Some(true) = complexity_finding {
        return Priority::Medium;
    }
    Priority::Low
}

fn parse_selection(entry: &Value) -> Option<Selection> {
    if let Some(selection_value) = entry.get("selection") {
        let start = selection_value.get("start")?;
        let end = selection_value.get("end")?;
        let start_line = start.get("line")?.as_i64()?;
        let start_char = start.get("character")?.as_i64()?;
        let end_line = end.get("line")?.as_i64()?;
        let end_char = end.get("character")?.as_i64()?;
        return Some(Selection {
            start: Position {
                line: start_line,
                character: start_char,
            },
            end: Position {
                line: end_line,
                character: end_char,
            },
        });
    }

    let start_line = entry.get("line_start").and_then(|v| v.as_i64());
    let end_line = entry.get("line_end").and_then(|v| v.as_i64());
    if let (Some(start_line), Some(end_line)) = (start_line, end_line) {
        return Some(Selection {
            start: Position {
                line: start_line,
                character: 1,
            },
            end: Position {
                line: end_line,
                character: 1,
            },
        });
    }

    None
}

#[cfg(test)]
mod tests {
    use super::build_prompt;

    #[test]
    fn build_prompt_points_agent_at_target_path() {
        let prompt = build_prompt("src/index.js");

        assert!(prompt.contains("Target file path (relative to current workspace): src/index.js"));
        assert!(prompt.contains("Inspect the target file from the current workspace"));
        assert!(prompt.contains("The summary must be specific to the target file"));
        assert!(prompt.contains("briefly name the checked categories that were absent"));
        assert!(prompt.contains("treat the review as a reachability review"));
        assert!(prompt.contains("uses the target as an opaque executable"));
    }

    #[test]
    fn build_prompt_does_not_embed_file_contents() {
        let prompt = build_prompt("data/labels.json");

        assert!(!prompt.contains("--- FILE CONTENTS ---"));
        assert!(!prompt.contains("review me"));
    }
}
