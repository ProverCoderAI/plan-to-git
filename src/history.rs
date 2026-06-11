use std::fs;
use std::path::{Path, PathBuf};

use crate::error::AppResult;
use crate::pr_body::{END_MARKER, START_MARKER};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryImportOutcome {
    pub files_scanned: usize,
    pub files_matched: usize,
    pub lines_scanned: usize,
    pub parse_errors: usize,
    pub plans_found: usize,
    pub plans_added: usize,
    pub duplicates: usize,
    pub rendered_stacks_skipped: usize,
}

impl HistoryImportOutcome {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            files_scanned: 0,
            files_matched: 0,
            lines_scanned: 0,
            parse_errors: 0,
            plans_found: 0,
            plans_added: 0,
            duplicates: 0,
            rendered_stacks_skipped: 0,
        }
    }
}

impl Default for HistoryImportOutcome {
    fn default() -> Self {
        Self::new()
    }
}

pub fn collect_jsonl_files(dir: &Path, files: &mut Vec<PathBuf>) -> AppResult<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_jsonl_files(&path, files)?;
        } else if path
            .extension()
            .is_some_and(|extension| extension == "jsonl")
        {
            files.push(path);
        }
    }
    Ok(())
}

#[must_use]
pub fn looks_like_rendered_plan_stack(content: &str) -> bool {
    content.contains("## Agent Plan Stack")
        && content.contains(START_MARKER)
        && content.contains(END_MARKER)
}

#[must_use]
pub fn session_id_from_path(path: &Path) -> Option<String> {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .map(ToOwned::to_owned)
}

#[must_use]
pub fn line_turn_id(path: &Path, line_number: usize) -> Option<String> {
    let stem = path.file_stem()?.to_str()?;
    Some(format!("{stem}:{line_number}"))
}
