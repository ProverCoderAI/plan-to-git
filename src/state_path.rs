use sha2::{Digest, Sha256};
use std::path::PathBuf;

use crate::git::GitContext;
use crate::store::STATE_FILE_NAME;

pub const STATE_DIR_ENV: &str = "PLAN_TO_GIT_STATE_DIR";
pub const STATE_PATH_ENV: &str = "PLAN_TO_GIT_STATE_PATH";

#[must_use]
pub fn state_path(context: &GitContext) -> PathBuf {
    if let Some(path) = std::env::var_os(STATE_PATH_ENV) {
        return PathBuf::from(path);
    }

    let state_dir = std::env::var_os(STATE_DIR_ENV)
        .map_or_else(|| std::env::temp_dir().join("plan-to-git"), PathBuf::from);
    state_dir.join(repo_key(context)).join(STATE_FILE_NAME)
}

fn repo_key(context: &GitContext) -> String {
    let label = context
        .repo_slug
        .as_deref()
        .or_else(|| context.repo_root.file_name().and_then(|name| name.to_str()))
        .unwrap_or("repo");
    let label = sanitize_label(label);
    let hash = hex_prefix(
        &Sha256::digest(context.repo_root.to_string_lossy().as_bytes()),
        12,
    );

    format!("{label}-{hash}")
}

fn sanitize_label(label: &str) -> String {
    let sanitized = label
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_owned();

    if sanitized.is_empty() {
        "repo".to_owned()
    } else {
        sanitized
    }
}

fn hex_prefix(bytes: &[u8], length: usize) -> String {
    bytes
        .iter()
        .flat_map(|byte| [byte >> 4, byte & 0x0f])
        .take(length)
        .map(|nibble| char::from(b"0123456789abcdef"[usize::from(nibble)]))
        .collect()
}
