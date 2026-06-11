use serde::Deserialize;
use serde_json::Value;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use crate::error::AppResult;
use crate::git;
use crate::github::{self, SyncStatus};
use crate::normalize::{extract_marked_plans, extract_questions, CapturedPlan};
use crate::state_path;
use crate::store::{
    load_state, save_state, AgentSource, NewDecision, NewPendingQuestion, NewPlanItem,
    PendingQuestion,
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
    #[serde(default, alias = "last_agent_message")]
    last_assistant_message: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ClaudeHookInput {
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    transcript_path: Option<PathBuf>,
    #[serde(default)]
    cwd: Option<PathBuf>,
    hook_event_name: String,
    #[serde(default)]
    prompt: Option<String>,
    #[serde(default, alias = "last_agent_message")]
    last_assistant_message: Option<String>,
}

pub fn process_codex_hook(input: &str) -> AppResult<HookOutcome> {
    let hook_input: CodexHookInput = serde_json::from_str(input)?;
    process_agent_hook(&AgentHookInput {
        source: AgentSource::Codex,
        session_id: hook_input.session_id,
        cwd: hook_input.cwd,
        hook_event_name: hook_input.hook_event_name,
        turn_id: hook_input.turn_id,
        prompt: hook_input.prompt,
        last_assistant_message: hook_input.last_assistant_message,
        transcript_path: None,
    })
}

pub fn process_claude_hook(input: &str) -> AppResult<HookOutcome> {
    let hook_input: ClaudeHookInput = serde_json::from_str(input)?;
    process_agent_hook(&AgentHookInput {
        source: AgentSource::Claude,
        session_id: hook_input.session_id,
        cwd: hook_input.cwd,
        hook_event_name: hook_input.hook_event_name,
        turn_id: None,
        prompt: hook_input.prompt,
        last_assistant_message: hook_input.last_assistant_message,
        transcript_path: hook_input.transcript_path,
    })
}

struct AgentHookInput {
    source: AgentSource,
    session_id: Option<String>,
    cwd: Option<PathBuf>,
    hook_event_name: String,
    turn_id: Option<String>,
    prompt: Option<String>,
    last_assistant_message: Option<String>,
    transcript_path: Option<PathBuf>,
}

fn process_agent_hook(hook_input: &AgentHookInput) -> AppResult<HookOutcome> {
    let start_dir = hook_input.cwd.as_deref().unwrap_or_else(|| Path::new("."));
    let context = git::discover(start_dir)?;
    let state_path = state_path::state_path(&context);
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
            let message = hook_input.last_assistant_message.clone().or_else(|| {
                hook_input
                    .transcript_path
                    .as_deref()
                    .and_then(last_assistant_message_from_transcript)
            });

            if let Some(message) = message.as_deref() {
                for plan in extract_marked_plans(message) {
                    let added = state.add_plan(NewPlanItem {
                        source: hook_input.source,
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
            }

            if hook_input.source == AgentSource::Claude && captured_plans == 0 {
                if let Some(plan) = hook_input
                    .transcript_path
                    .as_deref()
                    .and_then(last_claude_plan_mode_plan_from_transcript)
                {
                    let added = state.add_plan(NewPlanItem {
                        source: hook_input.source,
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
            }

            if captured_plans == 0 {
                if let Some(message) = message.as_deref() {
                    let questions = extract_questions(message);
                    if state.add_pending_question(NewPendingQuestion {
                        source: hook_input.source,
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
                        source: hook_input.source,
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

fn last_assistant_message_from_transcript(path: &Path) -> Option<String> {
    let file = File::open(path).ok()?;
    let reader = BufReader::new(file);
    let mut last_message = None;

    for line in reader.lines().map_while(Result::ok) {
        let Ok(event) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let Some(message) = claude_assistant_message_text(&event) else {
            continue;
        };
        last_message = Some(message);
    }

    last_message
}

fn last_claude_plan_mode_plan_from_transcript(path: &Path) -> Option<CapturedPlan> {
    let file = File::open(path).ok()?;
    let reader = BufReader::new(file);
    let mut last_plan = None;

    for line in reader.lines().map_while(Result::ok) {
        let Ok(event) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let Some(plan) = claude_plan_mode_plan(&event) else {
            continue;
        };
        last_plan = Some(plan);
    }

    last_plan
}

fn claude_assistant_message_text(event: &Value) -> Option<String> {
    if event.get("type").and_then(Value::as_str) != Some("assistant") {
        return None;
    }
    if event
        .get("message")
        .and_then(|message| message.get("role"))
        .and_then(Value::as_str)
        != Some("assistant")
    {
        return None;
    }

    claude_content_text(event.get("message")?.get("content")?)
}

fn claude_content_text(content: &Value) -> Option<String> {
    if let Some(text) = content.as_str() {
        return (!text.trim().is_empty()).then_some(text.to_owned());
    }

    let text = content
        .as_array()?
        .iter()
        .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|block| block.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("\n");

    (!text.trim().is_empty()).then_some(text)
}

fn claude_plan_mode_plan(event: &Value) -> Option<CapturedPlan> {
    event
        .get("toolUseResult")
        .and_then(|result| result.get("plan"))
        .and_then(Value::as_str)
        .and_then(captured_plan_from_markdown)
}

fn captured_plan_from_markdown(content: &str) -> Option<CapturedPlan> {
    let content = content.trim();
    if content.is_empty() {
        return None;
    }

    Some(CapturedPlan {
        title: markdown_title(content),
        content: content.to_owned(),
    })
}

fn markdown_title(content: &str) -> Option<String> {
    content
        .lines()
        .find_map(|line| line.trim().strip_prefix("# "))
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .map(ToOwned::to_owned)
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
