use chrono::{SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::Path;

use crate::error::AppResult;
use crate::redact::redact;

pub const STATE_FILE_NAME: &str = ".agent-plan.json";
const SCHEMA_VERSION: u8 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentPlanState {
    pub schema_version: u8,
    pub repo: Option<String>,
    pub branch: Option<String>,
    pub head_sha: Option<String>,
    #[serde(default)]
    pub items: Vec<PlanStackItem>,
    #[serde(default)]
    pub pending_questions: Vec<PendingQuestion>,
}

impl Default for AgentPlanState {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            repo: None,
            branch: None,
            head_sha: None,
            items: Vec::new(),
            pending_questions: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlanStackItem {
    pub id: String,
    pub kind: PlanItemKind,
    pub source: AgentSource,
    pub title: Option<String>,
    pub content: String,
    #[serde(default)]
    pub questions: Vec<String>,
    pub answer: Option<String>,
    pub branch: Option<String>,
    pub head_sha: Option<String>,
    pub session_id: Option<String>,
    pub turn_id: Option<String>,
    pub content_hash: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PlanItemKind {
    Plan,
    Decision,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentSource {
    Codex,
    Claude,
    Manual,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PendingQuestion {
    pub id: String,
    pub source: AgentSource,
    pub questions: Vec<String>,
    pub branch: Option<String>,
    pub head_sha: Option<String>,
    pub session_id: Option<String>,
    pub turn_id: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewPlanItem {
    pub source: AgentSource,
    pub title: Option<String>,
    pub content: String,
    pub branch: Option<String>,
    pub head_sha: Option<String>,
    pub session_id: Option<String>,
    pub turn_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewPendingQuestion {
    pub source: AgentSource,
    pub questions: Vec<String>,
    pub branch: Option<String>,
    pub head_sha: Option<String>,
    pub session_id: Option<String>,
    pub turn_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewDecision {
    pub source: AgentSource,
    pub questions: Vec<String>,
    pub answer: String,
    pub branch: Option<String>,
    pub head_sha: Option<String>,
    pub session_id: Option<String>,
    pub turn_id: Option<String>,
}

impl AgentPlanState {
    pub fn set_context(
        &mut self,
        repo: Option<String>,
        branch: Option<String>,
        head_sha: Option<String>,
    ) {
        self.repo = repo;
        self.branch = branch;
        self.head_sha = head_sha;
    }

    pub fn add_plan(&mut self, new_item: NewPlanItem) -> bool {
        let content = redact(new_item.content.trim());
        if content.is_empty() {
            return false;
        }

        let content_hash = item_hash(
            PlanItemKind::Plan,
            new_item.source,
            &content,
            &[],
            None,
            new_item.branch.as_deref(),
        );

        if self
            .items
            .iter()
            .any(|item| item.content_hash == content_hash)
        {
            return false;
        }

        self.items.push(PlanStackItem {
            id: item_id("plan", &content_hash),
            kind: PlanItemKind::Plan,
            source: new_item.source,
            title: new_item.title.map(|title| redact(title.trim())),
            content,
            questions: Vec::new(),
            answer: None,
            branch: new_item.branch,
            head_sha: new_item.head_sha,
            session_id: new_item.session_id,
            turn_id: new_item.turn_id,
            content_hash,
            created_at: timestamp(),
        });

        true
    }

    pub fn add_pending_question(&mut self, new_question: NewPendingQuestion) -> bool {
        let questions: Vec<String> = new_question
            .questions
            .into_iter()
            .map(|question| redact(question.trim()))
            .filter(|question| !question.is_empty())
            .collect();

        if questions.is_empty() {
            return false;
        }

        let question_hash = stable_hash(&questions.join("\n"));
        if self
            .pending_questions
            .iter()
            .any(|question| question.id == item_id("question", &question_hash))
        {
            return false;
        }

        self.pending_questions.push(PendingQuestion {
            id: item_id("question", &question_hash),
            source: new_question.source,
            questions,
            branch: new_question.branch,
            head_sha: new_question.head_sha,
            session_id: new_question.session_id,
            turn_id: new_question.turn_id,
            created_at: timestamp(),
        });

        true
    }

    pub fn answer_pending_questions(&mut self, new_decision: NewDecision) -> bool {
        let answer = redact(new_decision.answer.trim());
        if answer.is_empty() || new_decision.questions.is_empty() {
            return false;
        }

        let questions: Vec<String> = new_decision
            .questions
            .into_iter()
            .map(|question| redact(question.trim()))
            .filter(|question| !question.is_empty())
            .collect();

        if questions.is_empty() {
            return false;
        }

        let content = render_decision_content(&questions, &answer);
        let content_hash = item_hash(
            PlanItemKind::Decision,
            new_decision.source,
            &content,
            &questions,
            Some(&answer),
            new_decision.branch.as_deref(),
        );

        if self
            .items
            .iter()
            .any(|item| item.content_hash == content_hash)
        {
            self.pending_questions.clear();
            return false;
        }

        self.items.push(PlanStackItem {
            id: item_id("decision", &content_hash),
            kind: PlanItemKind::Decision,
            source: new_decision.source,
            title: Some(String::from("Planning decision")),
            content,
            questions,
            answer: Some(answer),
            branch: new_decision.branch,
            head_sha: new_decision.head_sha,
            session_id: new_decision.session_id,
            turn_id: new_decision.turn_id,
            content_hash,
            created_at: timestamp(),
        });
        self.pending_questions.clear();

        true
    }
}

pub fn load_state(path: &Path) -> AppResult<AgentPlanState> {
    if !path.exists() {
        return Ok(AgentPlanState::default());
    }

    let content = fs::read_to_string(path)?;
    let mut state: AgentPlanState = serde_json::from_str(&content)?;
    if state.schema_version == 0 {
        state.schema_version = SCHEMA_VERSION;
    }
    Ok(state)
}

pub fn save_state(path: &Path, state: &AgentPlanState) -> AppResult<()> {
    let content = serde_json::to_string_pretty(state)?;
    fs::write(path, format!("{content}\n"))?;
    Ok(())
}

fn render_decision_content(questions: &[String], answer: &str) -> String {
    let mut content = String::from("Questions:\n");
    for question in questions {
        content.push_str("- ");
        content.push_str(question);
        content.push('\n');
    }
    content.push_str("\nAnswer:\n");
    content.push_str(answer);
    content
}

fn item_hash(
    kind: PlanItemKind,
    source: AgentSource,
    content: &str,
    questions: &[String],
    answer: Option<&str>,
    branch: Option<&str>,
) -> String {
    let mut input = format!("{kind:?}\n{source:?}\n{}\n", content.trim());
    if !questions.is_empty() {
        input.push_str(&questions.join("\n"));
    }
    if let Some(answer) = answer {
        input.push_str(answer);
    }
    if let Some(branch) = branch {
        input.push_str(branch);
    }
    stable_hash(&input)
}

fn item_id(prefix: &str, hash: &str) -> String {
    let short_hash: String = hash.chars().take(12).collect();
    format!("{prefix}-{short_hash}")
}

fn stable_hash(input: &str) -> String {
    let digest = Sha256::digest(input.as_bytes());
    let mut output = String::with_capacity(digest.len() * 2);
    for byte in digest {
        output.push(hex_char(byte >> 4));
        output.push(hex_char(byte & 0x0f));
    }
    output
}

const fn hex_char(value: u8) -> char {
    match value {
        0..=9 => (b'0' + value) as char,
        _ => (b'a' + (value - 10)) as char,
    }
}

fn timestamp() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}
