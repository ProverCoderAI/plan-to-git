use plan_to_git::normalize::{extract_marked_plans, extract_questions};
use plan_to_git::pr_body::{upsert_marker_block, END_MARKER, START_MARKER};
use plan_to_git::redact::redact;
use plan_to_git::render::{has_current_branch_items, render_plan_block, render_plan_comment};
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
fn tagged_plan_ignores_inline_tag_examples() {
    let message = r"
<proposed_plan>
# Parser Plan

- Capture `<proposed_plan>...</proposed_plan>` examples as prose.
- Continue until the real closing marker.
</proposed_plan>
";

    let plans = extract_marked_plans(message);

    assert_eq!(plans.len(), 1);
    assert!(plans[0].content.contains("Continue until the real"));
}

#[test]
fn extracts_single_line_tagged_plan() {
    let message = "<proposed_plan># Inline\n- Keep it</proposed_plan>";

    let plans = extract_marked_plans(message);

    assert_eq!(plans.len(), 1);
    assert!(plans[0].content.contains("Keep it"));
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
fn render_escapes_nested_plan_markers() {
    let mut state = AgentPlanState::default();
    state.set_context(None, Some("feature/current".to_owned()), None);
    assert!(state.add_plan(NewPlanItem {
        source: AgentSource::Codex,
        title: Some("Markers".to_owned()),
        content: format!("inside {START_MARKER} and {END_MARKER}"),
        branch: Some("feature/current".to_owned()),
        head_sha: None,
        session_id: None,
        turn_id: None,
        created_at: None,
    }));

    let rendered = render_plan_block(&state);

    assert_eq!(rendered.matches(START_MARKER).count(), 1);
    assert_eq!(rendered.matches(END_MARKER).count(), 1);
    assert!(rendered.contains("&lt;!-- plan-to-git:start --&gt;"));
}

#[test]
fn state_tracks_commented_items_per_pr() {
    let mut state = AgentPlanState::default();
    state.set_context(None, Some("feature/current".to_owned()), None);
    assert!(state.add_plan(NewPlanItem {
        source: AgentSource::Codex,
        title: Some("Commented".to_owned()),
        content: "- Post once".to_owned(),
        branch: Some("feature/current".to_owned()),
        head_sha: None,
        session_id: None,
        turn_id: None,
        created_at: None,
    }));

    let item_ids = state
        .unposted_items_for_pr(17)
        .iter()
        .map(|item| item.id.clone())
        .collect::<Vec<_>>();
    let comment = render_plan_comment(&state, &state.unposted_items_for_pr(17));

    assert!(comment.contains("Agent Plan Update"));
    assert!(comment.contains("Post once"));
    state.mark_items_commented(17, &item_ids, Some(12345));

    assert!(state.unposted_items_for_pr(17).is_empty());
    assert_eq!(state.unposted_items_for_pr(18).len(), 1);
    assert_eq!(state.posted_comments[0].comment_id, Some(12345));
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
fn pr_body_replaces_through_last_marker() {
    let original =
        format!("Intro\n\n{START_MARKER}\nold\n{END_MARKER}\nstale\n{END_MARKER}\nOutro");
    let block = format!("{START_MARKER}\nnew\n{END_MARKER}");

    let replaced = upsert_marker_block(&original, &block).expect("replace should work");

    assert!(replaced.contains("Intro"));
    assert!(replaced.contains("new"));
    assert!(replaced.contains("Outro"));
    assert!(!replaced.contains("old"));
    assert!(!replaced.contains("stale"));
}

#[test]
fn pr_body_rejects_partial_markers() {
    let body = format!("Existing body\n\n{START_MARKER}\nmissing end");
    let block = format!("{START_MARKER}\n## Agent Plan Stack\n{END_MARKER}");

    assert!(upsert_marker_block(&body, &block).is_err());
}
