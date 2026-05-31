use plan_to_git::normalize::{extract_marked_plans, extract_questions};
use plan_to_git::pr_body::{upsert_marker_block, END_MARKER, START_MARKER};
use plan_to_git::redact::redact;
use plan_to_git::render::{has_current_branch_items, render_plan_block};
use plan_to_git::store::{
    AgentPlanState, AgentSource, NewDecision, NewPendingQuestion, NewPlanItem,
};

#[test]
fn extracts_proposed_plan_blocks() {
    let message = r"
before
<proposed_plan>
# Implement Plan

- Add capture.
- Add sync.
</proposed_plan>
after
";

    let plans = extract_marked_plans(message);

    assert_eq!(plans.len(), 1);
    assert_eq!(plans[0].title.as_deref(), Some("Implement Plan"));
    assert!(plans[0].content.contains("- Add capture."));
}

#[test]
fn extracts_accepted_plan_headings() {
    let message = r"
## Accepted Plan
- Capture marked plans only.
- Sync to PR markers.

## Summary
Do not include this.
";

    let plans = extract_marked_plans(message);

    assert_eq!(plans.len(), 1);
    assert_eq!(plans[0].title.as_deref(), Some("Accepted Plan"));
    assert!(!plans[0].content.contains("Summary"));
}

#[test]
fn extracts_accepted_plan_label_until_next_heading() {
    let message = r"
Accepted Plan:
- Keep this.

### Notes
Do not include this.
";

    let plans = extract_marked_plans(message);

    assert_eq!(plans.len(), 1);
    assert_eq!(plans[0].title.as_deref(), Some("Accepted Plan"));
    assert!(!plans[0].content.contains("Notes"));
}

#[test]
fn rejects_unmarked_plan_like_text() {
    let message = "Plan:\n- Read everything\n- Upload all context";

    assert!(extract_marked_plans(message).is_empty());
}

#[test]
fn extracts_assistant_questions() {
    let message = r"
I need two choices:
- Which agent should be first?
1. Should sync be automatic?
This line is not a question.
";

    let questions = extract_questions(message);

    assert_eq!(
        questions,
        vec![
            "Which agent should be first?".to_owned(),
            "Should sync be automatic?".to_owned()
        ]
    );
}

#[test]
fn redacts_common_secret_shapes() {
    let redacted = redact(
        "api_key=sk-abcdefghijklmnopqrstuvwxyz token: ghp_abcdefghijklmnopqrstuvwxyz ghp_abcdefghijklmnopqrstuvwxyz",
    );

    assert!(redacted.contains("api_key=[REDACTED]"));
    assert!(redacted.contains("token=[REDACTED]"));
    assert!(redacted.contains("[REDACTED_GITHUB_TOKEN]"));
    assert!(!redacted.contains("ghp_abcdefghijklmnopqrstuvwxyz"));
}

#[test]
fn state_deduplicates_plans_and_records_decisions() {
    let mut state = AgentPlanState::default();

    let first = state.add_plan(NewPlanItem {
        source: AgentSource::Codex,
        title: Some("Implement Plan".to_owned()),
        content: "- Step 1".to_owned(),
        branch: Some("feature".to_owned()),
        head_sha: Some("abc123".to_owned()),
        session_id: Some("session".to_owned()),
        turn_id: Some("turn".to_owned()),
        created_at: None,
    });
    let duplicate = state.add_plan(NewPlanItem {
        source: AgentSource::Codex,
        title: Some("Implement Plan".to_owned()),
        content: "- Step 1".to_owned(),
        branch: Some("feature".to_owned()),
        head_sha: Some("abc123".to_owned()),
        session_id: Some("session".to_owned()),
        turn_id: Some("turn".to_owned()),
        created_at: None,
    });

    assert!(first);
    assert!(!duplicate);
    assert_eq!(state.items.len(), 1);

    assert!(state.add_pending_question(NewPendingQuestion {
        source: AgentSource::Codex,
        questions: vec!["Auto sync?".to_owned()],
        branch: Some("feature".to_owned()),
        head_sha: Some("abc123".to_owned()),
        session_id: Some("session".to_owned()),
        turn_id: Some("turn-2".to_owned()),
    }));

    assert!(state.answer_pending_questions(NewDecision {
        source: AgentSource::Codex,
        questions: vec!["Auto sync?".to_owned()],
        answer: "Yes, sync automatically.".to_owned(),
        branch: Some("feature".to_owned()),
        head_sha: Some("abc123".to_owned()),
        session_id: Some("session".to_owned()),
        turn_id: Some("turn-3".to_owned()),
    }));

    assert_eq!(state.pending_questions.len(), 0);
    assert_eq!(state.items.len(), 2);
    assert!(state.items[1].content.contains("Yes, sync automatically."));
}

#[test]
fn render_filters_items_to_current_branch() {
    let mut state = AgentPlanState::default();
    state.set_context(
        Some("example/repo".to_owned()),
        Some("feature/current".to_owned()),
        Some("abcdef1234567890".to_owned()),
    );
    assert!(state.add_plan(NewPlanItem {
        source: AgentSource::Codex,
        title: Some("Current".to_owned()),
        content: "- Current branch plan".to_owned(),
        branch: Some("feature/current".to_owned()),
        head_sha: Some("abcdef1234567890".to_owned()),
        session_id: None,
        turn_id: None,
        created_at: None,
    }));
    assert!(state.add_plan(NewPlanItem {
        source: AgentSource::Codex,
        title: Some("Other".to_owned()),
        content: "- Other branch plan".to_owned(),
        branch: Some("feature/other".to_owned()),
        head_sha: Some("1234567890abcdef".to_owned()),
        session_id: None,
        turn_id: None,
        created_at: None,
    }));

    let rendered = render_plan_block(&state);

    assert!(has_current_branch_items(&state));
    assert!(rendered.contains("Current branch plan"));
    assert!(!rendered.contains("Other branch plan"));
}

#[test]
fn pr_body_appends_and_replaces_marker_block() {
    let original = "Existing body";
    let block = format!("{START_MARKER}\n## Agent Plan Stack\n{END_MARKER}");

    let appended = upsert_marker_block(original, &block).expect("append should work");
    assert!(appended.contains(original));
    assert!(appended.contains(START_MARKER));

    let replacement = format!("{START_MARKER}\n## Updated\n{END_MARKER}");
    let replaced = upsert_marker_block(&appended, &replacement).expect("replace should work");
    assert!(replaced.contains("## Updated"));
    assert!(!replaced.contains("## Agent Plan Stack"));
}

#[test]
fn pr_body_rejects_partial_markers() {
    let body = format!("Existing body\n\n{START_MARKER}\nmissing end");
    let block = format!("{START_MARKER}\n## Agent Plan Stack\n{END_MARKER}");

    assert!(upsert_marker_block(&body, &block).is_err());
}
