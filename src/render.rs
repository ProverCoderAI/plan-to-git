use crate::pr_body::{END_MARKER, START_MARKER};
use crate::store::{AgentPlanState, AgentSource, PlanItemKind, PlanStackItem};

#[must_use]
pub fn render_plan_block(state: &AgentPlanState) -> String {
    let mut output = String::new();
    output.push_str(START_MARKER);
    output.push('\n');
    output.push_str("## Agent Plan Stack\n\n");

    if let Some(branch) = &state.branch {
        output.push_str("_Branch: `");
        output.push_str(branch);
        output.push('`');
        if let Some(head_sha) = &state.head_sha {
            output.push_str(" at `");
            output.push_str(short_sha(head_sha));
            output.push('`');
        }
        output.push_str("._\n\n");
    }

    let items = current_branch_items(state);

    if items.is_empty() {
        output.push_str("_No captured plans yet._\n");
    } else {
        for (index, item) in items.iter().enumerate() {
            output.push_str("### ");
            output.push_str(&(index + 1).to_string());
            output.push_str(". ");
            output.push_str(item_title(item.kind));
            output.push('\n');
            output.push_str("Source: ");
            output.push_str(source_label(item.source));
            output.push_str(" - Captured: ");
            output.push_str(&item.created_at);
            output.push_str("\n\n");
            output.push_str(item.content.trim());
            output.push_str("\n\n");
        }
    }

    output.push_str(END_MARKER);
    output
}

#[must_use]
pub fn has_current_branch_items(state: &AgentPlanState) -> bool {
    state
        .items
        .iter()
        .any(|item| matches_current_branch(item, state.branch.as_deref()))
}

fn current_branch_items(state: &AgentPlanState) -> Vec<&PlanStackItem> {
    state
        .items
        .iter()
        .filter(|item| matches_current_branch(item, state.branch.as_deref()))
        .collect()
}

fn matches_current_branch(item: &PlanStackItem, current_branch: Option<&str>) -> bool {
    match (item.branch.as_deref(), current_branch) {
        (Some(item_branch), Some(branch)) => item_branch == branch,
        (Some(_), None) | (None, _) => true,
    }
}

const fn item_title(kind: PlanItemKind) -> &'static str {
    match kind {
        PlanItemKind::Plan => "Plan",
        PlanItemKind::Decision => "Planning Decision",
    }
}

const fn source_label(source: AgentSource) -> &'static str {
    match source {
        AgentSource::Codex => "codex",
        AgentSource::Claude => "claude",
        AgentSource::Manual => "manual",
    }
}

fn short_sha(sha: &str) -> &str {
    sha.get(..7).unwrap_or(sha)
}
