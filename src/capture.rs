use serde::Deserialize;
use std::path::{Path, PathBuf};

use crate::error::AppResult;
use crate::git;
use crate::github::{self, SyncStatus};
use crate::normalize::{extract_marked_plans, extract_questions};
use crate::store::{
    load_state, save_state, AgentSource, NewDecision, NewPendingQuestion, NewPlanItem,
    PendingQuestion, STATE_FILE_NAME,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookOutcome {
    pub changed: bool,
    pub captured_plans: usize,
    pub captured_decisions: usize,
    pub pending_questions: usize,
    pub sync_status: SyncStatus,
}

#[derive(Debug, Deserialize)]
struct CodexHookInput {
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    cwd: Option<PathBuf>,
    hook_event_name: String,
    #[serde(default)]
    turn_id: Option<String>,
    #[serde(default)]
    prompt: Option<String>,
    #[serde(default)]
    last_assistant_message: Option<String>,
}

pub fn process_codex_hook(input: &str) -> AppResult<HookOutcome> {
    let hook_input: CodexHookInput = serde_json::from_str(input)?;
    let start_dir = hook_input.cwd.as_deref().unwrap_or_else(|| Path::new("."));
    let context = git::discover(start_dir)?;
    let state_path = context.repo_root.join(STATE_FILE_NAME);
    let mut state = load_state(&state_path)?;
    state.set_context(
        context.repo_slug.clone(),
        context.branch.clone(),
        context.head_sha.clone(),
    );

    let mut captured_plans = 0;
    let mut captured_decisions = 0;
    let mut changed = false;

    match hook_input.hook_event_name.as_str() {
        "Stop" => {
            if let Some(message) = hook_input.last_assistant_message.as_deref() {
                for plan in extract_marked_plans(message) {
                    let added = state.add_plan(NewPlanItem {
                        source: AgentSource::Codex,
                        title: plan.title,
                        content: plan.content,
                        branch: context.branch.clone(),
                        head_sha: context.head_sha.clone(),
                        session_id: hook_input.session_id.clone(),
                        turn_id: hook_input.turn_id.clone(),
                        created_at: None,
                    });
                    if added {
                        captured_plans += 1;
                        changed = true;
                    }
                }

                if captured_plans == 0 {
                    let questions = extract_questions(message);
                    if state.add_pending_question(NewPendingQuestion {
                        source: AgentSource::Codex,
                        questions,
                        branch: context.branch.clone(),
                        head_sha: context.head_sha.clone(),
                        session_id: hook_input.session_id.clone(),
                        turn_id: hook_input.turn_id.clone(),
                    }) {
                        changed = true;
                    }
                }
            }
        }
        "UserPromptSubmit" => {
            if let Some(prompt) = hook_input.prompt.as_deref() {
                if !prompt.trim().is_empty() {
                    let questions = drain_relevant_questions(&mut state.pending_questions);
                    if state.answer_pending_questions(NewDecision {
                        source: AgentSource::Codex,
                        questions,
                        answer: prompt.to_owned(),
                        branch: context.branch.clone(),
                        head_sha: context.head_sha.clone(),
                        session_id: hook_input.session_id.clone(),
                        turn_id: hook_input.turn_id.clone(),
                    }) {
                        captured_decisions += 1;
                        changed = true;
                    }
                }
            }
        }
        _ => {}
    }

    if changed || !state.items.is_empty() || !state.pending_questions.is_empty() {
        save_state(&state_path, &state)?;
    }

    let sync_status = github::sync_state(&context, &mut state)?;
    if changed || !state.items.is_empty() || !state.pending_questions.is_empty() {
        save_state(&state_path, &state)?;
    }

    Ok(HookOutcome {
        changed,
        captured_plans,
        captured_decisions,
        pending_questions: state.pending_questions.len(),
        sync_status,
    })
}

fn drain_relevant_questions(pending_questions: &mut Vec<PendingQuestion>) -> Vec<String> {
    let mut questions = Vec::new();
    for pending_question in pending_questions.drain(..) {
        for question in pending_question.questions {
            if !questions.iter().any(|existing| existing == &question) {
                questions.push(question);
            }
        }
    }
    questions
}
