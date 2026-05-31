use crate::error::{AppError, AppResult};

pub const START_MARKER: &str = "<!-- plan-to-git:start -->";
pub const END_MARKER: &str = "<!-- plan-to-git:end -->";

pub fn upsert_marker_block(body: &str, block: &str) -> AppResult<String> {
    let start = body.find(START_MARKER);
    let end = body.rfind(END_MARKER);

    match (start, end) {
        (None, None) => Ok(append_block(body, block)),
        (Some(start_index), Some(end_index)) if start_index < end_index => {
            let replace_end = end_index + END_MARKER.len();
            let mut updated = String::new();
            updated.push_str(body[..start_index].trim_end());
            updated.push_str("\n\n");
            updated.push_str(block.trim());
            updated.push_str("\n\n");
            updated.push_str(body[replace_end..].trim_start());
            Ok(updated.trim_end().to_owned())
        }
        (Some(_), Some(_)) => Err(AppError::new("plan-to-git PR markers are out of order").into()),
        _ => Err(AppError::new("plan-to-git PR body has only one marker").into()),
    }
}

fn append_block(body: &str, block: &str) -> String {
    if body.trim().is_empty() {
        return block.trim().to_owned();
    }

    format!("{}\n\n{}", body.trim_end(), block.trim())
}
