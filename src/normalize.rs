#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapturedPlan {
    pub title: Option<String>,
    pub content: String,
}

#[must_use]
pub fn extract_marked_plans(message: &str) -> Vec<CapturedPlan> {
    let mut plans = extract_tagged_plans(message);
    plans.extend(extract_accepted_plan_headings(message));
    plans
        .into_iter()
        .filter(|plan| !plan.content.trim().is_empty())
        .collect()
}

#[must_use]
pub fn extract_questions(message: &str) -> Vec<String> {
    let mut questions = Vec::new();
    for line in message.lines() {
        let candidate = strip_list_marker(line.trim());
        if !candidate.ends_with('?') || candidate.len() < 4 {
            continue;
        }

        let question = candidate.trim().trim_matches('`').trim();
        if question.len() > 240 {
            continue;
        }

        if !questions.iter().any(|existing| existing == question) {
            questions.push(question.to_owned());
        }

        if questions.len() == 10 {
            break;
        }
    }
    questions
}

fn extract_tagged_plans(message: &str) -> Vec<CapturedPlan> {
    let mut plans = Vec::new();
    let mut content_lines: Option<Vec<&str>> = None;

    for line in message.lines() {
        let trimmed = line.trim();
        if trimmed.eq_ignore_ascii_case("<proposed_plan>") {
            content_lines = Some(Vec::new());
            continue;
        }

        if trimmed.eq_ignore_ascii_case("</proposed_plan>") {
            let Some(lines) = content_lines.take() else {
                continue;
            };
            let content = lines.join("\n").trim().to_owned();
            if content.is_empty() {
                continue;
            }
            plans.push(CapturedPlan {
                title: first_heading(&content),
                content,
            });
            continue;
        }

        if let Some(lines) = content_lines.as_mut() {
            lines.push(line);
        }
    }

    plans
}

fn extract_accepted_plan_headings(message: &str) -> Vec<CapturedPlan> {
    let lines: Vec<&str> = message.lines().collect();
    let mut plans = Vec::new();
    let mut index = 0;

    while index < lines.len() {
        let trimmed = lines[index].trim();
        if !is_accepted_plan_heading(trimmed) {
            index += 1;
            continue;
        }

        let current_heading_level = heading_level(trimmed).unwrap_or(6);
        let title = clean_heading(trimmed);
        let mut content_lines = Vec::new();
        index += 1;

        while index < lines.len() {
            let next = lines[index].trim();
            if heading_level(next).is_some_and(|next_level| next_level <= current_heading_level) {
                break;
            }
            content_lines.push(lines[index]);
            index += 1;
        }

        let content = content_lines.join("\n").trim().to_owned();
        if !content.is_empty() {
            plans.push(CapturedPlan {
                title: Some(title),
                content,
            });
        }
    }

    plans
}

fn is_accepted_plan_heading(line: &str) -> bool {
    let normalized = clean_heading(line).to_lowercase();
    matches!(
        normalized.as_str(),
        "accepted plan" | "принятый план" | "актуальный план"
    )
}

fn clean_heading(line: &str) -> String {
    line.trim_start_matches('#')
        .trim()
        .trim_end_matches(':')
        .trim()
        .to_owned()
}

fn first_heading(content: &str) -> Option<String> {
    content
        .lines()
        .map(str::trim)
        .find(|line| heading_level(line).is_some())
        .map(clean_heading)
}

fn heading_level(line: &str) -> Option<usize> {
    let hashes = line
        .chars()
        .take_while(|character| *character == '#')
        .count();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    line.chars()
        .nth(hashes)
        .is_some_and(char::is_whitespace)
        .then_some(hashes)
}

fn strip_list_marker(line: &str) -> &str {
    let without_bullet = line
        .strip_prefix("- ")
        .or_else(|| line.strip_prefix("* "))
        .unwrap_or(line);

    let Some((marker, rest)) = without_bullet.split_once(". ") else {
        return without_bullet;
    };

    if marker.chars().all(|character| character.is_ascii_digit()) {
        rest
    } else {
        without_bullet
    }
}
