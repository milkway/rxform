//! Post-parse validation: reports mistakes with their spreadsheet location
//! (sheet / row / column) and a probable cause, before XML generation runs.

use std::collections::BTreeMap;

use crate::error::{did_you_mean, Error, Result};
use crate::model::*;

pub fn validate(survey: &Survey) -> Result<()> {
    let multi = survey.xpath_multi_index();
    let names = survey.xpath_index();
    let ambiguous: std::collections::BTreeSet<&str> = multi
        .iter()
        .filter(|(_, paths)| paths.len() > 1)
        .map(|(name, _)| name.as_str())
        .collect();
    check_names(survey)?;
    check_selects(survey)?;
    check_choice_duplicates(survey)?;
    check_references(survey, &names, &ambiguous)?;
    check_entity(survey, &names, &ambiguous)?;
    check_triggers(survey, &names)?;
    // last, so more specific diagnostics take precedence
    check_labels(survey)?;
    Ok(())
}

/// The entities sheet may reference survey fields; `save_to` cannot be used
/// inside repeats (each entity property holds a single value).
fn check_entity(
    survey: &Survey,
    names: &BTreeMap<String, String>,
    ambiguous: &std::collections::BTreeSet<&str>,
) -> Result<()> {
    if let Some(entity) = &survey.entity {
        for (column, value) in [
            ("entity_id", &entity.entity_id),
            ("create_if", &entity.create_if),
            ("update_if", &entity.update_if),
            ("label", &entity.label),
        ] {
            if let Some(value) = value {
                scan_refs(value, names, ambiguous)
                    .map_err(|e| e.sheet("entities").row(entity.row).column(column))?;
            }
        }
    }
    fn saveto_in_repeat(items: &[Item], inside_repeat: bool) -> Option<usize> {
        for item in items {
            match item {
                Item::Question(q) => {
                    if inside_repeat && q.bind_extra.contains_key("entities:saveto") {
                        return Some(q.row);
                    }
                }
                Item::Section(s) => {
                    let nested = inside_repeat || s.kind == SectionKind::Repeat;
                    if let Some(row) = saveto_in_repeat(&s.children, nested) {
                        return Some(row);
                    }
                }
            }
        }
        None
    }
    if let Some(row) = saveto_in_repeat(&survey.items, false) {
        return Err(
            Error::new("save_to is not allowed on questions inside a repeat")
                .sheet("survey")
                .row(row)
                .column("save_to")
                .hint("an entity property holds a single value, but a repeat produces many"),
        );
    }
    Ok(())
}

/// Duplicate choice names within a list are rejected unless the settings
/// sheet opts in with `allow_choice_duplicates` (mirrors pyxform).
fn check_choice_duplicates(survey: &Survey) -> Result<()> {
    if survey.settings.allow_choice_duplicates {
        return Ok(());
    }
    for list in &survey.choice_lists {
        let mut seen: BTreeMap<&str, usize> = BTreeMap::new();
        for choice in &list.choices {
            if let Some(first) = seen.insert(choice.name.as_str(), choice.row) {
                return Err(Error::new(format!(
                    "the choice name '{}' appears more than once in list '{}' \
                     (rows {first} and {})",
                    choice.name, list.name, choice.row
                ))
                .sheet("choices")
                .row(choice.row)
                .column("name")
                .hint(
                    "choice names must be unique within a list; if this is \
                     intentional, add allow_choice_duplicates=yes to the settings sheet",
                ));
            }
        }
    }
    Ok(())
}

/// Every user-visible question must have a label or a hint, matching
/// pyxform's "has no label or hint" error — a control the user cannot read
/// is almost always an authoring mistake.
fn check_labels(survey: &Survey) -> Result<()> {
    for (_, q) in survey.walk() {
        // generated table-list header selects are label-less by design
        if q.name.starts_with("reserved_name_for_field_list_labels_") {
            continue;
        }
        let visible = crate::types::lookup(&q.qtype)
            .and_then(|d| d.control_tag)
            .is_some_and(|tag| tag != "action")
            || q.select.is_some();
        let has_builtin_hint = crate::types::lookup(&q.qtype)
            .map(|d| d.hint.is_some())
            .unwrap_or(false);
        if visible
            && q.label.is_empty()
            && q.hint.is_empty()
            && q.guidance_hint.is_empty()
            && q.media.is_empty()
            && !has_builtin_hint
        {
            return Err(
                Error::new(format!("the question '{}' has no label or hint", q.name))
                    .sheet("survey")
                    .row(q.row)
                    .column("label")
                    .hint(
                        "every visible question needs a label or hint; if it should stay \
                 hidden from the user, use type 'calculate' or 'hidden' instead",
                    ),
            );
        }
    }
    Ok(())
}

fn at(q_row: usize) -> Error {
    Error::new("").sheet("survey").row(q_row)
}

/// Element names become XML tags and xpath steps, so they must be valid
/// XML names without namespace colons.
fn is_valid_xml_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '.'))
}

fn check_names(survey: &Survey) -> Result<()> {
    // Duplicate names among SIBLINGS are always an error (their instance
    // nodes would collide). The same name in different sections is allowed
    // (loops produce that), but referencing it with ${...} is ambiguous —
    // that is caught at the reference site.
    fn walk(items: &[Item]) -> Result<()> {
        let mut seen: BTreeMap<&str, usize> = BTreeMap::new();
        for item in items {
            let (name, row, what) = match item {
                Item::Question(q) => (q.name.as_str(), q.row, "question"),
                Item::Section(s) => (s.name.as_str(), s.row, "group/repeat"),
            };
            if !is_valid_xml_name(name) {
                return Err(Error::new(format!("invalid {what} name '{name}'"))
                    .sheet("survey")
                    .row(row)
                    .column("name")
                    .hint(
                        "names must start with a letter or underscore and contain \
                         only letters, digits, '-', '_' and '.' (no spaces or accents-only symbols)",
                    ));
            }
            if let Some(first) = seen.insert(name, row) {
                return Err(Error::new(format!(
                    "the name '{name}' is used more than once (rows {first} and {row})"
                ))
                .sheet("survey")
                .row(row)
                .column("name")
                .hint("names must be unique so that ${reference} lookups are unambiguous"));
            }
            if let Item::Section(s) = item {
                walk(&s.children)?;
            }
        }
        Ok(())
    }
    walk(&survey.items)
}

fn check_selects(survey: &Survey) -> Result<()> {
    let list_names: Vec<&str> = survey
        .choice_lists
        .iter()
        .map(|l| l.name.as_str())
        .collect();
    for (_, q) in survey.walk() {
        let Some((kind, list_name)) = &q.select else {
            continue;
        };
        if q.qtype == "select one external" || q.select_from_file().is_some() {
            continue;
        }
        let Some(list) = survey.choice_list(list_name) else {
            return Err(at(q.row)
                .column("type")
                .hint_or_lists(list_name, &list_names)
                .message(format!(
                    "question '{}' references choice list '{list_name}', which does \
                     not exist in the choices sheet",
                    q.name
                )));
        };
        if *kind == SelectKind::Multiple {
            for choice in &list.choices {
                if choice.name.contains(' ') {
                    return Err(Error::new(format!(
                        "choice name '{}' in list '{list_name}' contains a space, \
                         which select_multiple cannot represent",
                        choice.name
                    ))
                    .sheet("choices")
                    .row(choice.row)
                    .column("name")
                    .hint(
                        "selected values are stored space-separated, so multiple-choice \
                         names must not contain spaces; use '_' instead",
                    ));
                }
            }
        }
    }
    Ok(())
}

/// Small helper so the select error reads fluently above.
trait SelectErrorExt {
    fn hint_or_lists(self, input: &str, lists: &[&str]) -> Self;
    fn message(self, message: String) -> Error;
}

impl SelectErrorExt for Error {
    fn hint_or_lists(self, input: &str, lists: &[&str]) -> Self {
        match did_you_mean(input, lists.iter().copied()) {
            Some(hint) => self.hint(hint),
            None if lists.is_empty() => {
                self.hint("the workbook has no choices sheet (or it is empty)")
            }
            None => self.hint(format!("available lists: {}", lists.join(", "))),
        }
    }

    fn message(mut self, message: String) -> Error {
        self.message = message;
        self
    }
}

/// Scan one expression for `${name}` (and `${last-saved#name}`) references,
/// without location info — callers attach sheet/row/column.
fn scan_refs(
    value: &str,
    names: &BTreeMap<String, String>,
    ambiguous: &std::collections::BTreeSet<&str>,
) -> Result<()> {
    let mut rest = value;
    while let Some(start) = rest.find("${") {
        let after = &rest[start + 2..];
        let Some(end) = after.find('}') else {
            return Err(
                Error::new(format!("unterminated ${{...}} reference in '{value}'"))
                    .hint("add the closing '}'"),
            );
        };
        let name = after[..end].trim();
        let lookup = name
            .strip_prefix("last-saved#")
            .map(str::trim)
            .unwrap_or(name);
        if ambiguous.contains(lookup) {
            return Err(Error::new(format!(
                "'${{{lookup}}}' is ambiguous: more than one field has this name"
            ))
            .hint("rename one of the fields so the reference is unambiguous"));
        }
        if !names.contains_key(lookup) {
            return Err(Error::new(format!(
                "'${{{name}}}' does not match the name of any question or group"
            ))
            .maybe_hint(did_you_mean(lookup, names.keys().map(String::as_str))));
        }
        rest = &after[end + 1..];
    }
    Ok(())
}

/// Every expression column that may contain `${name}` references, checked
/// against the actual field names so typos are caught with a suggestion.
fn check_references(
    survey: &Survey,
    names: &BTreeMap<String, String>,
    ambiguous: &std::collections::BTreeSet<&str>,
) -> Result<()> {
    let scan = |value: &str, row: usize, column: &str| -> Result<()> {
        scan_refs(value, names, ambiguous).map_err(|e| e.sheet("survey").row(row).column(column))
    };

    fn walk(items: &[Item], scan: &dyn Fn(&str, usize, &str) -> Result<()>) -> Result<()> {
        for item in items {
            match item {
                Item::Question(q) => {
                    let fields: [(&str, Option<&String>); 6] = [
                        ("relevant", q.relevant.as_ref()),
                        ("constraint", q.constraint.as_ref()),
                        ("calculation", q.calculation.as_ref()),
                        ("required", q.required.as_ref()),
                        ("choice_filter", q.choice_filter.as_ref()),
                        ("appearance", q.appearance.as_ref()),
                    ];
                    for (column, value) in fields {
                        if let Some(value) = value {
                            scan(value, q.row, column)?;
                        }
                    }
                    if q.default_is_dynamic() {
                        if let Some(default) = &q.default {
                            scan(default, q.row, "default")?;
                        }
                    }
                    for value in q.bind_extra.values() {
                        scan(value, q.row, "bind::*")?;
                    }
                    for tr in [&q.label, &q.hint].into_iter().chain([
                        &q.guidance_hint,
                        &q.constraint_message,
                        &q.required_message,
                    ]) {
                        for value in tr.values() {
                            scan(value, q.row, "label/hint")?;
                        }
                    }
                }
                Item::Section(s) => {
                    if let Some(relevant) = &s.relevant {
                        scan(relevant, s.row, "relevant")?;
                    }
                    if let Some(count) = &s.count {
                        scan(count, s.row, "repeat_count")?;
                    }
                    for value in s.label.values() {
                        scan(value, s.row, "label")?;
                    }
                    walk(&s.children, scan)?;
                }
            }
        }
        Ok(())
    }
    walk(&survey.items, &scan)?;

    if let Some(expr) = &survey.settings.instance_name {
        scan_refs(expr, names, ambiguous)
            .map_err(|e| e.sheet("settings").column("instance_name"))?;
    }
    Ok(())
}

fn check_triggers(survey: &Survey, names: &BTreeMap<String, String>) -> Result<()> {
    for (_, q) in survey.walk() {
        if q.qtype == "background-geopoint" && q.trigger.is_none() {
            return Err(Error::new(format!(
                "background-geopoint question '{}' has no trigger",
                q.name
            ))
            .sheet("survey")
            .row(q.row)
            .column("trigger")
            .hint("set the trigger column to ${question} — the location is captured when that question's value changes"));
        }
        let Some(trigger) = &q.trigger else { continue };
        let inner = trigger.trim();
        let name = inner
            .strip_prefix("${")
            .and_then(|s| s.strip_suffix('}'))
            .map(str::trim);
        let Some(name) = name else {
            return Err(Error::new(format!(
                "trigger '{inner}' is not a ${{question}} reference"
            ))
            .sheet("survey")
            .row(q.row)
            .column("trigger")
            .hint("the trigger column must contain exactly one reference, e.g. ${age}"));
        };
        if !names.contains_key(name) {
            return Err(Error::new(format!(
                "trigger references '${{{name}}}', which does not match any question"
            ))
            .sheet("survey")
            .row(q.row)
            .column("trigger")
            .maybe_hint(did_you_mean(name, names.keys().map(String::as_str))));
        }
        // The trigger source must be user-visible: its control hosts the
        // setvalue action, so a bodyless question can never fire it.
        let source = survey.walk().into_iter().find(|(_, s)| s.name == name);
        if let Some((_, source)) = source {
            let bodyless = matches!(source.qtype.as_str(), "calculate" | "hidden")
                || crate::types::lookup(&source.qtype)
                    .map(|d| d.control_tag.is_none())
                    .unwrap_or(false);
            if bodyless {
                return Err(Error::new(format!(
                    "the question ${{{name}}} is not user-visible, so it cannot be \
                     used as a trigger for '{}'",
                    q.name
                ))
                .sheet("survey")
                .row(q.row)
                .column("trigger")
                .hint("triggers must point at a question the user answers; calculations never 'change' interactively"));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xml_name_rules() {
        assert!(is_valid_xml_name("q1"));
        assert!(is_valid_xml_name("_x-y.z"));
        assert!(!is_valid_xml_name("1q"));
        assert!(!is_valid_xml_name("my name"));
        assert!(!is_valid_xml_name(""));
    }
}
