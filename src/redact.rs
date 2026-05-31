use regex::Regex;

#[must_use]
pub fn redact(input: &str) -> String {
    let mut output = input.to_owned();
    for (pattern, replacement) in secret_patterns() {
        if let Ok(regex) = Regex::new(pattern) {
            output = regex.replace_all(&output, replacement).into_owned();
        }
    }
    output
}

const fn secret_patterns() -> [(&'static str, &'static str); 7] {
    [
        (
            r#"(?i)(api[_-]?key|token|secret|password|authorization)\s*[:=]\s*['"]?[^'"\s`]+"#,
            "$1=[REDACTED]",
        ),
        (r"(?i)bearer\s+[a-z0-9._\-]{16,}", "Bearer [REDACTED]"),
        (r"sk-[a-zA-Z0-9_\-]{16,}", "[REDACTED_OPENAI_KEY]"),
        (r"ghp_[a-zA-Z0-9_]{20,}", "[REDACTED_GITHUB_TOKEN]"),
        (r"github_pat_[a-zA-Z0-9_]{20,}", "[REDACTED_GITHUB_TOKEN]"),
        (r"xox[baprs]-[a-zA-Z0-9\-]{20,}", "[REDACTED_SLACK_TOKEN]"),
        (r"AKIA[0-9A-Z]{16}", "[REDACTED_AWS_KEY]"),
    ]
}
