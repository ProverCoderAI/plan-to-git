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
    let close_tag = "</proposed_plan>";
    let mut cursor = 0;
    let mut plans = Vec::new();

    while let Some(open_tag) = find_proposed_plan_open_tag(message, cursor) {
        let content_start = open_tag.end;
        let mut close_cursor = content_start;

        let Some(content_end) = (loop {
            let Some(candidate_start) =
                find_ascii_case_insensitive(message, close_tag, close_cursor)
            else {
                break None;
            };
            let candidate_end = candidate_start + close_tag.len();
            if closes_plan_block(message, candidate_end) {
                break Some(candidate_start);
            }
            close_cursor = candidate_end;
        }) else {
            break;
        };

        let content = message[content_start..content_end].trim();
        if !content.is_empty() {
            let content_heading = first_heading(content);
            let has_content_heading = content_heading.is_some();
            let title = content_heading.or(open_tag.title);
            let content = if has_content_heading {
                content.to_owned()
            } else {
                title.as_deref().map_or_else(
                    || content.to_owned(),
                    |title| format!("# {title}\n\n{content}"),
                )
            };
            let content = normalize_xml_plan_sections(&content);
            plans.push(CapturedPlan { title, content });
        }
        cursor = content_end + close_tag.len();
    }

    plans
}

struct ProposedPlanOpenTag {
    end: usize,
    title: Option<String>,
}

fn find_proposed_plan_open_tag(message: &str, cursor: usize) -> Option<ProposedPlanOpenTag> {
    const OPEN_TAG_START: &str = "<proposed_plan";

    let mut search_start = cursor;
    while let Some(tag_start) = find_ascii_case_insensitive(message, OPEN_TAG_START, search_start) {
        let after_name = tag_start + OPEN_TAG_START.len();
        let next_character = message[after_name..].chars().next()?;

        if next_character != '>' && !next_character.is_ascii_whitespace() {
            search_start = after_name;
            continue;
        }

        let tag_end = after_name + message[after_name..].find('>')? + 1;
        let tag = &message[tag_start..tag_end];
        let title = attribute_value(tag, "title")
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);

        return Some(ProposedPlanOpenTag {
            end: tag_end,
            title,
        });
    }

    None
}

fn attribute_value<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let mut index = "<proposed_plan".len();

    while index < tag.len() {
        let rest = &tag[index..];
        let Some(character) = rest.chars().next() else {
            break;
        };

        if character == '>' {
            break;
        }

        if character.is_ascii_whitespace() {
            index += character.len_utf8();
            continue;
        }

        let attribute_name_start = index;
        while index < tag.len() {
            let Some(character) = tag[index..].chars().next() else {
                break;
            };
            if !matches!(character, '-' | '_' | ':') && !character.is_ascii_alphanumeric() {
                break;
            }
            index += character.len_utf8();
        }
        if attribute_name_start == index {
            if let Some(character) = tag[index..].chars().next() {
                index += character.len_utf8();
            }
            continue;
        }

        let attribute_name = &tag[attribute_name_start..index];
        while index < tag.len() {
            let Some(character) = tag[index..].chars().next() else {
                break;
            };
            if !character.is_ascii_whitespace() {
                break;
            }
            index += character.len_utf8();
        }

        if !tag[index..].starts_with('=') {
            continue;
        }
        index += 1;

        while index < tag.len() {
            let Some(character) = tag[index..].chars().next() else {
                break;
            };
            if !character.is_ascii_whitespace() {
                break;
            }
            index += character.len_utf8();
        }

        let Some(quote) = tag[index..].chars().next() else {
            break;
        };
        if quote != '"' && quote != '\'' {
            let value_start = index;
            while index < tag.len() {
                let Some(character) = tag[index..].chars().next() else {
                    break;
                };
                if character == '>' || character.is_ascii_whitespace() {
                    break;
                }
                index += character.len_utf8();
            }
            if attribute_name.eq_ignore_ascii_case(name) {
                return Some(&tag[value_start..index]);
            }
            continue;
        }

        index += quote.len_utf8();
        let value_start = index;
        let Some(value_end_relative) = tag[index..].find(quote) else {
            break;
        };
        let value_end = index + value_end_relative;
        index = value_end + quote.len_utf8();

        if attribute_name.eq_ignore_ascii_case(name) {
            return Some(&tag[value_start..value_end]);
        }
    }

    None
}

fn find_ascii_case_insensitive(haystack: &str, needle: &str, from: usize) -> Option<usize> {
    if needle.is_empty() || from > haystack.len() || needle.len() > haystack.len() {
        return None;
    }

    let haystack = haystack.as_bytes();
    let needle = needle.as_bytes();
    for index in from..=haystack.len().saturating_sub(needle.len()) {
        if haystack[index..index + needle.len()]
            .iter()
            .zip(needle.iter())
            .all(|(candidate, expected)| candidate.eq_ignore_ascii_case(expected))
        {
            return Some(index);
        }
    }

    None
}

fn closes_plan_block(message: &str, close_tag_end: usize) -> bool {
    message[close_tag_end..]
        .lines()
        .next()
        .is_none_or(|rest_of_line| rest_of_line.trim().is_empty())
}

fn normalize_xml_plan_sections(content: &str) -> String {
    let lines = content.lines().collect::<Vec<_>>();
    let mut output = Vec::new();
    let mut index = 0;
    let mut changed = false;
    let mut in_code_fence = false;
    let mut details_depth = 0usize;

    while index < lines.len() {
        if is_code_fence_line(lines[index]) {
            in_code_fence = !in_code_fence;
            output.push(lines[index].to_owned());
            index += 1;
            continue;
        }

        if in_code_fence {
            output.push(lines[index].to_owned());
            index += 1;
            continue;
        }

        if is_open_details_line(lines[index]) {
            details_depth += 1;
            output.push(lines[index].to_owned());
            index += 1;
            continue;
        }

        if is_close_details_line(lines[index]) {
            details_depth = details_depth.saturating_sub(1);
            output.push(lines[index].to_owned());
            index += 1;
            continue;
        }

        if details_depth > 0 {
            output.push(lines[index].to_owned());
            index += 1;
            continue;
        }

        let Some(section) = xml_plan_section_opening(lines[index]) else {
            output.push(lines[index].to_owned());
            index += 1;
            continue;
        };

        let Some(close_index) = lines[index + 1..]
            .iter()
            .position(|line| xml_plan_section_closing(line, section.tag))
            .map(|relative_index| index + 1 + relative_index)
        else {
            output.push(lines[index].to_owned());
            index += 1;
            continue;
        };

        trim_trailing_blank_lines(&mut output);
        if !output.is_empty() {
            output.push(String::new());
        }
        output.push(format!("## {}", section.heading));

        let body = dedent_lines(&lines[index + 1..close_index]);
        if !body.is_empty() {
            output.push(String::new());
            output.extend(body);
        }

        index = close_index + 1;
        changed = true;
    }

    if changed {
        trim_trailing_blank_lines(&mut output);
        output.join("\n")
    } else {
        content.to_owned()
    }
}

struct XmlPlanSection {
    tag: &'static str,
    heading: &'static str,
}

fn xml_plan_section_opening(line: &str) -> Option<XmlPlanSection> {
    const SECTIONS: &[XmlPlanSection] = &[
        XmlPlanSection {
            tag: "summary",
            heading: "Summary",
        },
        XmlPlanSection {
            tag: "flow",
            heading: "Flow",
        },
        XmlPlanSection {
            tag: "test_plan",
            heading: "Test Plan",
        },
        XmlPlanSection {
            tag: "assumptions",
            heading: "Assumptions",
        },
    ];

    let trimmed = line.trim();
    SECTIONS
        .iter()
        .find(|section| trimmed.eq_ignore_ascii_case(&format!("<{}>", section.tag)))
        .map(|section| XmlPlanSection {
            tag: section.tag,
            heading: section.heading,
        })
}

fn xml_plan_section_closing(line: &str, tag: &str) -> bool {
    line.trim().eq_ignore_ascii_case(&format!("</{tag}>"))
}

fn dedent_lines(lines: &[&str]) -> Vec<String> {
    let minimum_indent = lines
        .iter()
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.len() - line.trim_start().len())
        .min()
        .unwrap_or(0);

    let mut dedented = lines
        .iter()
        .map(|line| {
            if line.trim().is_empty() {
                String::new()
            } else {
                line.get(minimum_indent..).unwrap_or(line).to_owned()
            }
        })
        .collect::<Vec<_>>();

    while dedented.first().is_some_and(String::is_empty) {
        dedented.remove(0);
    }
    trim_trailing_blank_lines(&mut dedented);
    dedented
}

fn trim_trailing_blank_lines(lines: &mut Vec<String>) {
    while lines.last().is_some_and(|line| line.trim().is_empty()) {
        lines.pop();
    }
}

fn is_code_fence_line(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with("```") || trimmed.starts_with("~~~")
}

fn is_open_details_line(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.eq_ignore_ascii_case("<details>")
        || (trimmed.len() > "<details>".len()
            && trimmed
                .get(.."<details ".len())
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case("<details "))
            && trimmed.ends_with('>'))
}

fn is_close_details_line(line: &str) -> bool {
    line.trim().eq_ignore_ascii_case("</details>")
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
