use plan_to_git::render::render_plan_block;
use plan_to_git::store::{AgentPlanState, AgentSource, NewPlanItem};

fn main() {
    let mut state = AgentPlanState::default();
    state.set_context(
        Some("example/repo".to_owned()),
        Some("feature/plan-sync".to_owned()),
        Some("abcdef1234567890".to_owned()),
    );
    state.add_plan(NewPlanItem {
        source: AgentSource::Codex,
        title: Some("Example Plan".to_owned()),
        content: "- Capture the plan.\n- Sync it to the pull request.".to_owned(),
        branch: Some("feature/plan-sync".to_owned()),
        head_sha: Some("abcdef1234567890".to_owned()),
        session_id: None,
        turn_id: None,
    });

    println!("{}", render_plan_block(&state));
}
