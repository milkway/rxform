//! Parses generic sheet data (survey/choices/settings) into a [`Survey`] tree.

use std::collections::BTreeMap;

use crate::error::{did_you_mean, Error, Result};
use crate::model::*;
use crate::types;
use crate::xls::{Sheet, Workbook};

mod validate;

/// Parse a workbook into a survey. `fallback_name` (usually the file stem)
/// supplies the form name/id/title when the settings sheet does not.
pub fn parse(workbook: &Workbook, fallback_name: &str) -> Result<Survey> {
    // clean_text_values defaults to on: collapse runs of spaces and trim
    // every cell (smart quotes are already straightened at read time)
    let workbook = match clean_text_enabled(&workbook.settings) {
        true => {
            let mut wb = workbook.clone();
            for sheet in [
                &mut wb.survey,
                &mut wb.choices,
                &mut wb.settings,
                &mut wb.external_choices,
                &mut wb.entities,
            ] {
                *sheet = clean_sheet_whitespace(sheet);
            }
            wb
        }
        false => workbook.clone(),
    };
    let workbook = &workbook;
    let settings = parse_settings(&workbook.settings);
    let mut choice_lists = parse_choices(&workbook.choices)?;
    let mut items = parse_survey(&workbook.survey)?;
    apply_or_other(&mut items, &mut choice_lists)?;
    apply_loops(&mut items, &choice_lists)?;
    if settings.flat {
        mark_groups_flat(&mut items);
    }

    let id_string = settings
        .form_id
        .clone()
        .unwrap_or_else(|| fallback_name.to_string());
    // Like current pyxform/xls2xform, the primary instance root element
    // defaults to "data" unless the settings sheet names it.
    let name = settings.name.clone().unwrap_or_else(|| "data".to_string());
    // Like pyxform, an explicit form_id doubles as the default title.
    let title = settings
        .form_title
        .clone()
        .or_else(|| settings.form_id.clone())
        .unwrap_or_else(|| fallback_name.to_string());

    let survey = Survey {
        name,
        id_string,
        title,
        settings,
        items,
        choice_lists,
        external_choices: parse_external_choices(&workbook.external_choices),
        entity: parse_entities(&workbook.entities)?,
    };
    validate::validate(&survey)?;
    Ok(survey)
}

/// Keep the external_choices sheet verbatim (headers + cells, in order); it
/// is exported as itemsets.csv rather than woven into the XForm.
fn parse_external_choices(sheet: &Sheet) -> Option<ExternalChoices> {
    if sheet.headers.is_empty() || sheet.rows.is_empty() {
        return None;
    }
    let rows = sheet
        .rows
        .iter()
        .map(|(_, row)| {
            sheet
                .headers
                .iter()
                .map(|h| row.get(h).cloned().unwrap_or_default())
                .collect()
        })
        .collect();
    Some(ExternalChoices {
        headers: sheet.headers.clone(),
        rows,
    })
}

/// Expand `or_other` selects: append the Other choice to the list and insert
/// a "Specify other." text question right after the select.
fn apply_or_other(items: &mut Vec<Item>, choice_lists: &mut [ChoiceList]) -> Result<()> {
    let mut i = 0;
    while i < items.len() {
        match &mut items[i] {
            Item::Section(section) => {
                apply_or_other(&mut section.children, choice_lists)?;
                i += 1;
            }
            Item::Question(q) if q.or_other => {
                let at = |e: Error| e.sheet("survey").row(q.row).column("type");
                if q.choice_filter.is_some() {
                    return Err(at(Error::new(
                        "choice_filter is not supported together with or_other",
                    )
                    .hint(
                        "remove the choice_filter, or drop 'or_other' and model the \
                         Other choice explicitly in the choices sheet",
                    )));
                }
                if q.select_from_file().is_some() {
                    return Err(at(Error::new(
                        "or_other cannot be combined with select_*_from_file",
                    )));
                }
                let (_, list_name) = q.select.clone().expect("or_other implies a select");
                let Some(list) = choice_lists.iter_mut().find(|l| l.name == list_name) else {
                    return Err(at(Error::new(format!(
                        "please specify choices for this 'or other' question \
                         (list '{list_name}' not found)"
                    ))));
                };
                if !list.choices.iter().any(|c| c.name == "other") {
                    // With translated lists, "Other" is used for every language.
                    let langs: std::collections::BTreeSet<String> = list
                        .choices
                        .iter()
                        .flat_map(|c| c.label.keys().cloned())
                        .collect();
                    let mut label = Translated::new();
                    if langs.iter().all(|l| l == DEFAULT_LANG) || langs.is_empty() {
                        label.insert(DEFAULT_LANG.to_string(), "Other".to_string());
                    } else {
                        for lang in langs {
                            label.insert(lang, "Other".to_string());
                        }
                    }
                    list.choices.push(Choice {
                        name: "other".to_string(),
                        label,
                        media: BTreeMap::new(),
                        extras: Vec::new(),
                        row: 0,
                    });
                }
                let specify = Question {
                    qtype: "text".into(),
                    name: format!("{}_other", q.name),
                    label: {
                        let mut tr = Translated::new();
                        tr.insert(DEFAULT_LANG.to_string(), "Specify other.".to_string());
                        tr
                    },
                    relevant: Some(format!("selected(../{}, 'other')", q.name)),
                    row: q.row,
                    ..Question::default()
                };
                items.insert(i + 1, Item::Question(Box::new(specify)));
                i += 2;
            }
            Item::Question(_) => i += 1,
        }
    }
    Ok(())
}

fn kind_name(kind: SectionKind) -> &'static str {
    match kind {
        SectionKind::Group => "group",
        SectionKind::Repeat => "repeat",
        SectionKind::Loop => "loop",
    }
}

/// `clean_text_values` defaults to yes; read it from the raw settings sheet
/// before any other parsing happens.
fn clean_text_enabled(settings: &Sheet) -> bool {
    let Some((_, row)) = settings.rows.first() else {
        return true;
    };
    for (key, value) in row {
        if key.trim().to_lowercase() == "clean_text_values" {
            return !matches!(
                value.trim().to_lowercase().as_str(),
                "no" | "false" | "false()"
            );
        }
    }
    true
}

/// Collapse runs of spaces to one space and trim each cell, like pyxform's
/// clean_text_values (tabs and newlines inside the text are preserved).
fn clean_sheet_whitespace(sheet: &Sheet) -> Sheet {
    let mut cleaned = sheet.clone();
    for (_, row) in &mut cleaned.rows {
        for value in row.values_mut() {
            let collapsed: String = {
                let mut out = String::with_capacity(value.len());
                let mut prev_space = false;
                for c in value.trim().chars() {
                    if c == ' ' {
                        if !prev_space {
                            out.push(c);
                        }
                        prev_space = true;
                    } else {
                        prev_space = false;
                        out.push(c);
                    }
                }
                out
            };
            *value = collapsed;
        }
    }
    cleaned
}

/// Expand `begin loop over <list>` sections: one sub-group per choice, with
/// `%(label)s` / `%(name)s` substituted into the copied questions.
fn apply_loops(items: &mut [Item], choice_lists: &[ChoiceList]) -> Result<()> {
    for item in items.iter_mut() {
        if let Item::Section(section) = item {
            apply_loops(&mut section.children, choice_lists)?;
            if section.kind != SectionKind::Loop {
                continue;
            }
            let list_name = section.loop_list.clone().unwrap_or_default();
            let Some(list) = choice_lists.iter().find(|l| l.name == list_name) else {
                return Err(Error::new(format!(
                    "the loop over '{list_name}' references a choice list that does \
                     not exist in the choices sheet"
                ))
                .sheet("survey")
                .row(section.row)
                .column("type")
                .maybe_hint(did_you_mean(
                    &list_name,
                    choice_lists.iter().map(|l| l.name.as_str()),
                )));
            };
            let body = std::mem::take(&mut section.children);
            for choice in &list.choices {
                let sub = Section {
                    kind: SectionKind::Group,
                    name: choice.name.clone(),
                    label: choice.label.clone(),
                    hint: Translated::new(),
                    relevant: None,
                    appearance: None,
                    count: None,
                    children: body
                        .iter()
                        .map(|item| substitute_loop_item(item, choice))
                        .collect(),
                    row: section.row,
                    loop_list: None,
                    flat: false,
                };
                section.children.push(Item::Section(sub));
            }
            section.kind = SectionKind::Group;
            section.loop_list = None;
        }
    }
    Ok(())
}

/// Copy a loop-body item for one choice, substituting `%(name)s` and
/// `%(label)s` (per language) into its translatable text.
fn substitute_loop_item(item: &Item, choice: &Choice) -> Item {
    fn substitute(tr: &Translated, choice: &Choice) -> Translated {
        tr.iter()
            .map(|(lang, text)| {
                let label = choice
                    .label
                    .get(lang)
                    .or_else(|| choice.label.get(DEFAULT_LANG))
                    .or_else(|| choice.label.values().next())
                    .cloned()
                    .unwrap_or_default();
                let text = text
                    .replace("%(name)s", &choice.name)
                    .replace("%(label)s", &label);
                (lang.clone(), text)
            })
            .collect()
    }
    match item {
        Item::Question(q) => {
            let mut q = q.clone();
            q.label = substitute(&q.label, choice);
            q.hint = substitute(&q.hint, choice);
            Item::Question(q)
        }
        Item::Section(sec) => {
            let mut sec = sec.clone();
            sec.label = substitute(&sec.label, choice);
            sec.children = sec
                .children
                .iter()
                .map(|c| substitute_loop_item(c, choice))
                .collect();
            Item::Section(sec)
        }
    }
}

/// Under `settings flat=yes`, every group loses its instance node and xpath
/// segment (repeats are unaffected), and section relevance formulas are
/// "and"-ed down onto their leaf questions, like pyxform's flat annotations.
fn mark_groups_flat(items: &mut [Item]) {
    fn combine(parent: &str, own: Option<String>) -> Option<String> {
        match (parent.is_empty(), own) {
            (true, own) => own,
            (false, Some(own)) => Some(format!("{parent} and ({own})")),
            (false, None) => Some(parent.to_string()),
        }
    }
    fn rec(items: &mut [Item], parent_relevant: &str) {
        for item in items {
            match item {
                Item::Question(q) => {
                    q.relevant = combine(parent_relevant, q.relevant.take());
                }
                Item::Section(section) => {
                    if section.kind == SectionKind::Group {
                        section.flat = true;
                    }
                    let inherited =
                        combine(parent_relevant, section.relevant.take()).unwrap_or_default();
                    rec(&mut section.children, &inherited);
                }
            }
        }
    }
    rec(items, "");
}

/// Parse the `entities` sheet into a single entity declaration, enforcing
/// pyxform's rules about the entity_id / create_if / update_if combinations.
fn parse_entities(sheet: &Sheet) -> Result<Option<EntityDecl>> {
    if sheet.rows.is_empty() {
        return Ok(None);
    }
    if sheet.rows.len() > 1 {
        let (row, _) = sheet.rows[1];
        return Err(Error::new("only one entity can be declared per form")
            .sheet("entities")
            .row(row)
            .hint("remove the extra rows; a form may create/update a single entity"));
    }
    let (row_number, row) = &sheet.rows[0];
    let mut decl = EntityDecl {
        dataset: String::new(),
        entity_id: None,
        create_if: None,
        update_if: None,
        label: None,
        row: *row_number,
    };
    for (key, value) in row {
        let value = value.trim().to_string();
        match key.trim().to_lowercase().as_str() {
            "list_name" | "list name" | "dataset" => decl.dataset = value,
            "entity_id" => decl.entity_id = Some(value),
            "create_if" => decl.create_if = Some(value),
            "update_if" => decl.update_if = Some(value),
            "label" => decl.label = Some(value),
            other => {
                return Err(Error::new(format!(
                    "unexpected column '{other}' on the entities sheet"
                ))
                .sheet("entities")
                .row(*row_number)
                .maybe_hint(did_you_mean(
                    other,
                    ["list_name", "entity_id", "create_if", "update_if", "label"],
                )));
            }
        }
    }
    let at = |e: Error| e.sheet("entities").row(*row_number);
    if decl.dataset.is_empty() {
        return Err(at(Error::new("the entities sheet row has no list_name")
            .column("list_name")
            .hint(
                "list_name (or dataset) names the entity list this form writes to",
            )));
    }
    if decl.dataset.starts_with("__") || decl.dataset.contains('.') {
        return Err(at(Error::new(format!(
            "invalid entity list name '{}'",
            decl.dataset
        ))
        .column("list_name")
        .hint("entity list names cannot start with '__' or contain '.'")));
    }
    if decl.entity_id.is_none() && decl.update_if.is_some() {
        return Err(at(Error::new(
            "update_if requires an entity_id to know which entity to update",
        )
        .column("update_if")));
    }
    if decl.entity_id.is_some() && decl.create_if.is_some() && decl.update_if.is_none() {
        return Err(at(Error::new(
            "entity_id combined with create_if also needs update_if",
        )
        .column("create_if")
        .hint(
            "an entity_id alone means update; to conditionally create AND update, \
             provide both create_if and update_if",
        )));
    }
    if decl.entity_id.is_none() && decl.label.is_none() {
        return Err(at(Error::new(
            "a form that creates entities must give them a label",
        )
        .column("label")));
    }
    Ok(Some(decl))
}

// ---------------------------------------------------------------------------
// Header handling
// ---------------------------------------------------------------------------

/// A parsed column header: canonical base name + optional language.
struct Header {
    base: String,
    lang: Option<String>,
}

/// Columns whose header may carry a language tag (`label::Lang`, or the
/// legacy single-colon `label:Lang`).
const TRANSLATABLE_COLUMNS: [&str; 10] = [
    "label",
    "hint",
    "guidance_hint",
    "constraint_message",
    "constraining_message",
    "required_message",
    "image",
    "big-image",
    "audio",
    "video",
];

/// Split `label::English (en)` into ("label", Some("English (en)")); strip the
/// legacy `media::` prefix from `media::image::Lang`. Also accepts the old
/// single-colon form `label:english`.
fn parse_header(raw: &str) -> Header {
    let raw = raw.trim();
    let mut parts: Vec<&str> = raw.split("::").map(str::trim).collect();
    if parts.len() > 1 && parts[0].eq_ignore_ascii_case("media") {
        parts.remove(0);
    }
    let mut base = parts[0].to_lowercase();
    let mut lang = if parts.len() > 1 {
        Some(parts[1..].join("::"))
    } else {
        None
    };
    if lang.is_none() {
        if let Some((prefix, rest)) = parts[0].split_once(':') {
            let prefix_lower = prefix.trim().to_lowercase();
            if TRANSLATABLE_COLUMNS.contains(&prefix_lower.as_str()) && !rest.trim().is_empty() {
                base = prefix_lower;
                lang = Some(rest.trim().to_string());
            }
        }
    }
    Header { base, lang }
}

/// Canonical names for survey-sheet columns (pyxform's aliases.survey_header).
fn survey_column_alias(base: &str) -> &str {
    match base {
        "caption" => "label",
        "relevance" => "relevant",
        "read_only" => "readonly",
        "constraining_message" | "constraint_message" => "constraint_message",
        "calculate" | "calculation" => "calculation",
        "requiredmsg" | "required_message" => "required_message",
        "command" => "type",
        "tag" | "value" => "name",
        "count" | "jr:count" | "repeat_count" => "repeat_count",
        other => other,
    }
}

const MEDIA_KINDS: [&str; 4] = ["image", "big-image", "audio", "video"];

fn set_translated(map: &mut Translated, lang: &Option<String>, value: &str) {
    let key = lang.clone().unwrap_or_else(|| DEFAULT_LANG.to_string());
    map.insert(key, value.to_string());
}

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

fn parse_settings(sheet: &Sheet) -> Settings {
    let mut s = Settings::default();
    let Some((_, row)) = sheet.rows.first() else {
        return s;
    };
    for (key, value) in row {
        let value = &value.trim().to_string();
        let key_lower = key.trim().to_lowercase();
        if key_lower.starts_with("attribute:") {
            let rest = key_lower
                .strip_prefix("attribute::")
                .or_else(|| key_lower.strip_prefix("attribute:"))
                .unwrap_or_default();
            if !rest.is_empty() {
                s.attributes.push((original_tail(key, rest), value.clone()));
            }
            continue;
        }
        match parse_header(key).base.as_str() {
            "form_title" | "set_form_title" | "title" => s.form_title = Some(value.clone()),
            "form_id" | "set_form_id" | "id_string" => s.form_id = Some(value.clone()),
            "name" => s.name = Some(value.clone()),
            "version" => s.version = Some(value.clone()),
            "default_language" => s.default_language = Some(value.clone()),
            "instance_name" => s.instance_name = Some(value.clone()),
            "style" => s.style = Some(value.clone()),
            "submission_url" => s.submission_url = Some(value.clone()),
            "public_key" => s.public_key = Some(value.clone()),
            "auto_send" => s.auto_send = Some(value.clone()),
            "auto_delete" => s.auto_delete = Some(value.clone()),
            "allow_choice_duplicates" => {
                s.allow_choice_duplicates =
                    matches!(value.to_lowercase().as_str(), "yes" | "true" | "true()")
            }
            "namespaces" => s.namespaces = Some(value.clone()),
            "flat" => s.flat = matches!(value.to_lowercase().as_str(), "yes" | "true" | "true()"),
            "omit_instanceid" => {
                s.omit_instance_id =
                    matches!(value.to_lowercase().as_str(), "yes" | "true" | "true()")
            }
            "prefix" => s.attributes.push(("odk:prefix".into(), value.clone())),
            "delimiter" => s.attributes.push(("odk:delimiter".into(), value.clone())),
            _ => {}
        }
    }
    s
}

// ---------------------------------------------------------------------------
// Choices
// ---------------------------------------------------------------------------

fn parse_choices(sheet: &Sheet) -> Result<Vec<ChoiceList>> {
    let mut lists: Vec<ChoiceList> = Vec::new();
    for (row_number, row) in &sheet.rows {
        let mut list_name: Option<String> = None;
        let mut choice = Choice {
            name: String::new(),
            label: Translated::new(),
            media: BTreeMap::new(),
            extras: Vec::new(),
            row: *row_number,
        };
        // Iterate in original column order so extras keep their order.
        for header_raw in &sheet.headers {
            let Some(value) = row.get(header_raw) else {
                continue;
            };
            let h = parse_header(header_raw);
            let base = match h.base.as_str() {
                "caption" => "label",
                "value" => "name",
                b => b,
            };
            match base {
                "list_name" | "list name" => list_name = Some(value.trim().to_string()),
                "name" => choice.name = value.trim().to_string(),
                "label" => set_translated(&mut choice.label, &h.lang, value),
                m if MEDIA_KINDS.contains(&m) => set_translated(
                    choice.media.entry(m.to_string()).or_default(),
                    &h.lang,
                    value,
                ),
                _ => choice.extras.push((h.base.clone(), value.clone())),
            }
        }
        let list_name = list_name.ok_or_else(|| {
            Error::new("choice row has no list_name")
                .sheet("choices")
                .row(*row_number)
                .column("list_name")
                .hint("every choices row needs a list_name linking it to a select question")
        })?;
        if choice.name.is_empty() {
            return Err(
                Error::new(format!("choice in list '{list_name}' has no name"))
                    .sheet("choices")
                    .row(*row_number)
                    .column("name")
                    .hint("the 'name' column holds the value saved when this choice is selected"),
            );
        }
        match lists.iter_mut().find(|l| l.name == list_name) {
            Some(list) => list.choices.push(choice),
            None => lists.push(ChoiceList {
                name: list_name,
                choices: vec![choice],
            }),
        }
    }
    Ok(lists)
}

// ---------------------------------------------------------------------------
// Survey sheet
// ---------------------------------------------------------------------------

/// Parsed `type` cell.
enum RowType {
    Question {
        qtype: String,
        select: Option<(SelectKind, String)>,
        or_other: bool,
    },
    BeginSection(SectionKind, Option<String>),
    EndSection(SectionKind),
}

fn parse_type_cell(cell: &str) -> Option<RowType> {
    let mut cell = cell.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut or_other = false;
    for suffix in [" or specify other", " or_other", " or other"] {
        if cell.to_lowercase().ends_with(suffix) {
            cell.truncate(cell.len() - suffix.len());
            or_other = true;
            break;
        }
    }
    let lower = cell.to_lowercase();

    for (prefix, kind) in [
        ("begin group", SectionKind::Group),
        ("begin_group", SectionKind::Group),
        ("begin repeat", SectionKind::Repeat),
        ("begin_repeat", SectionKind::Repeat),
        ("begin lgroup", SectionKind::Repeat),
        ("begin looped group", SectionKind::Repeat),
    ] {
        if lower == prefix {
            return Some(RowType::BeginSection(kind, None));
        }
    }
    for prefix in ["begin loop over", "begin_loop over"] {
        if let Some(rest) = lower.strip_prefix(prefix) {
            let list = cell[prefix.len()..].trim().to_string();
            let _ = rest;
            if !list.is_empty() {
                return Some(RowType::BeginSection(SectionKind::Loop, Some(list)));
            }
        }
    }
    for (prefix, kind) in [
        ("end group", SectionKind::Group),
        ("end_group", SectionKind::Group),
        ("end repeat", SectionKind::Repeat),
        ("end_repeat", SectionKind::Repeat),
        ("end lgroup", SectionKind::Repeat),
        ("end looped group", SectionKind::Repeat),
        ("end loop", SectionKind::Loop),
        ("end_loop", SectionKind::Loop),
    ] {
        if lower == prefix || lower.starts_with(&format!("{prefix} ")) {
            return Some(RowType::EndSection(kind));
        }
    }

    // Select types: alias prefix + list name (or filename for the
    // `_from_file` variants). Longest prefixes first.
    let select_aliases: [(&str, SelectKind, &str); 14] = [
        (
            "select_multiple_from_file",
            SelectKind::Multiple,
            "select all that apply",
        ),
        (
            "select multiple from file",
            SelectKind::Multiple,
            "select all that apply",
        ),
        (
            "select all that apply from",
            SelectKind::Multiple,
            "select all that apply",
        ),
        (
            "select all that apply",
            SelectKind::Multiple,
            "select all that apply",
        ),
        (
            "select_multiple",
            SelectKind::Multiple,
            "select all that apply",
        ),
        (
            "select multiple",
            SelectKind::Multiple,
            "select all that apply",
        ),
        (
            "select_one_external",
            SelectKind::One,
            "select one external",
        ),
        (
            "select one external",
            SelectKind::One,
            "select one external",
        ),
        ("select_one_from_file", SelectKind::One, "select one"),
        ("select one from file", SelectKind::One, "select one"),
        ("select one from", SelectKind::One, "select one"),
        ("select_one", SelectKind::One, "select one"),
        ("select one", SelectKind::One, "select one"),
        ("select1", SelectKind::One, "select one"),
    ];
    if lower.starts_with("rank ") {
        let list = cell[5..].trim().to_string();
        return Some(RowType::Question {
            qtype: "rank".into(),
            select: Some((SelectKind::Rank, list)),
            or_other,
        });
    }
    for (alias, kind, canonical) in select_aliases {
        if lower.starts_with(&format!("{alias} ")) {
            let rest = cell[alias.len()..].trim().to_string();
            return Some(RowType::Question {
                qtype: canonical.to_string(),
                select: Some((kind, rest)),
                or_other,
            });
        }
    }

    // Plain types, via the type alias map then the type dictionary.
    let canonical = match lower.as_str() {
        "add photo prompt" | "add image prompt" => "photo",
        "add audio prompt" => "audio",
        "add video prompt" => "video",
        "add file prompt" => "file",
        other => other,
    };
    types::lookup(canonical).map(|_| RowType::Question {
        qtype: canonical.to_string(),
        select: None,
        or_other,
    })
}

fn parse_parameters(cell: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for pair in cell.split([';', ',', ' ']) {
        if let Some((k, v)) = pair.split_once('=') {
            let (k, v) = (k.trim(), v.trim());
            if !k.is_empty() && !v.is_empty() {
                out.insert(k.to_string(), v.to_string());
            }
        }
    }
    out
}

/// True when the whole value is a single `${name}` reference.
fn is_pure_reference(value: &str) -> bool {
    let v = value.trim();
    v.starts_with("${") && v.ends_with('}') && !v[2..v.len() - 1].contains('$')
}

/// `yes`/`no` and variants → `true()`/`false()`; expressions pass through.
fn binding_value(value: &str) -> String {
    match value.to_lowercase().as_str() {
        "yes" | "true" | "true()" => "true()".to_string(),
        "no" | "false" | "false()" => "false()".to_string(),
        _ => value.to_string(),
    }
}

fn parse_survey(sheet: &Sheet) -> Result<Vec<Item>> {
    let mut root: Vec<Item> = Vec::new();
    // Stack of open sections; the tree is assembled as sections close.
    let mut stack: Vec<Section> = Vec::new();

    let err = |row: usize, message: String| Error::new(message).sheet("survey").row(row);

    for (row_number, row) in &sheet.rows {
        let type_cell = row
            .iter()
            .find(|(k, _)| survey_column_alias(&parse_header(k).base) == "type")
            .map(|(_, v)| v.clone());
        let Some(type_cell) = type_cell else { continue };

        let row_type = parse_type_cell(&type_cell).ok_or_else(|| {
            err(
                *row_number,
                format!("unknown question type '{}'", type_cell.trim()),
            )
            .column("type")
            .maybe_hint(did_you_mean(
                type_cell.split_whitespace().next().unwrap_or(""),
                types::SUGGESTIBLE_TYPES,
            ))
        })?;

        match row_type {
            RowType::EndSection(kind) => {
                let mut section = stack.pop().ok_or_else(|| {
                    err(
                        *row_number,
                        format!("'{}' without a matching 'begin'", type_cell.trim()),
                    )
                    .column("type")
                    .hint("a group/repeat above may have been closed twice, or its 'begin' row deleted")
                })?;
                apply_table_list(&mut section)?;
                if section.kind != kind {
                    let (found, open) = (kind_name(kind), kind_name(section.kind));
                    return Err(err(
                        *row_number,
                        format!(
                            "'end {found}' does not match the open '{open}' \
                             named '{}' started on row {}",
                            section.name, section.row
                        ),
                    )
                    .column("type")
                    .hint(format!(
                        "change this row to 'end {open}', or fix the 'begin' on row {}",
                        section.row
                    )));
                }
                match stack.last_mut() {
                    Some(parent) => parent.children.push(Item::Section(section)),
                    None => root.push(Item::Section(section)),
                }
            }
            RowType::BeginSection(kind, loop_list) => {
                let q = parse_question_row(sheet, row, *row_number)?;
                if q.name.is_empty() {
                    return Err(err(*row_number, "group/repeat has no name".into())
                        .column("name")
                        .hint("every begin group/repeat row needs a value in the 'name' column"));
                }
                let mut count = q.bind_extra.get("repeat_count").cloned();
                if let Some(expr) = &count {
                    // A count expression that isn't a lone ${ref} gets its own
                    // generated calculate node so xpaths can reference it.
                    if !is_pure_reference(expr) {
                        let generated = format!("{}_count", q.name);
                        let helper = Question {
                            qtype: "calculate".into(),
                            name: generated.clone(),
                            calculation: Some(expr.clone()),
                            readonly: Some("true()".into()),
                            row: *row_number,
                            ..Question::default()
                        };
                        match stack.last_mut() {
                            Some(parent) => parent.children.push(Item::Question(Box::new(helper))),
                            None => root.push(Item::Question(Box::new(helper))),
                        }
                        count = Some(format!("${{{generated}}}"));
                    }
                }
                stack.push(Section {
                    kind,
                    name: q.name,
                    label: q.label,
                    hint: q.hint,
                    relevant: q.relevant,
                    appearance: q.appearance,
                    count,
                    children: Vec::new(),
                    row: *row_number,
                    loop_list,
                    flat: false,
                });
            }
            RowType::Question {
                qtype,
                select,
                or_other,
            } => {
                let mut q = parse_question_row(sheet, row, *row_number)?;
                q.qtype = qtype;
                q.select = select;
                q.or_other = or_other;
                if q.name.is_empty() {
                    if q.qtype == "note" {
                        // pyxform auto-names label-only notes by row number
                        q.name = format!("generated_note_name_{row_number}");
                    } else {
                        return Err(err(*row_number, "question has no name".into())
                            .column("name")
                            .hint("every question row needs a unique value in the 'name' column (notes are the only exception)"));
                    }
                }
                match stack.last_mut() {
                    Some(parent) => parent.children.push(Item::Question(Box::new(q))),
                    None => root.push(Item::Question(Box::new(q))),
                }
            }
        }
    }

    if let Some(open) = stack.last() {
        return Err(Error::new(format!(
            "the survey sheet ended with an unclosed {} named '{}'",
            kind_name(open.kind),
            open.name
        ))
        .sheet("survey")
        .row(open.row)
        .hint(format!(
            "add an 'end {}' row after the section's last question",
            kind_name(open.kind)
        )));
    }
    Ok(root)
}

/// pyxform's "table-list" appearance sugar: the group becomes a field-list;
/// its label/hint move into a generated leading note; a label-only copy of
/// the first select is inserted to render the column headers; and every
/// select gets the "list-nolabel" appearance. All selects must share one
/// choice list.
fn apply_table_list(section: &mut Section) -> Result<()> {
    let Some(appearance) = &section.appearance else {
        return Ok(());
    };
    let mods: Vec<&str> = appearance.split_whitespace().collect();
    if !mods.contains(&"table-list") {
        return Ok(());
    }
    let mut new_appearance = String::from("field-list");
    for m in &mods {
        if *m != "table-list" {
            new_appearance.push(' ');
            new_appearance.push_str(m);
        }
    }
    section.appearance = Some(new_appearance);

    let mut children = Vec::with_capacity(section.children.len() + 2);
    if !section.label.is_empty() || !section.hint.is_empty() {
        let mut note = Question {
            qtype: "note".into(),
            name: format!("generated_table_list_label_{}", section.row),
            row: section.row,
            ..Question::default()
        };
        note.label = std::mem::take(&mut section.label);
        note.hint = std::mem::take(&mut section.hint);
        children.push(Item::Question(Box::new(note)));
    }

    let mut table_list: Option<String> = None;
    for item in section.children.drain(..) {
        match item {
            Item::Question(mut q) if q.select.is_some() => {
                let (_, list_name) = q.select.clone().unwrap();
                match &table_list {
                    None => {
                        if q.choice_filter.is_some() {
                            return Err(Error::new(format!(
                                "choice_filter is not supported inside a group with \
                                 table-list appearance (question '{}')",
                                q.name
                            ))
                            .sheet("survey")
                            .row(q.row)
                            .column("choice_filter"));
                        }
                        let header = Question {
                            qtype: q.qtype.clone(),
                            name: format!("reserved_name_for_field_list_labels_{}", q.row),
                            select: q.select.clone(),
                            appearance: Some("label".into()),
                            row: q.row,
                            ..Question::default()
                        };
                        children.push(Item::Question(Box::new(header)));
                        table_list = Some(list_name);
                    }
                    Some(current) if *current != list_name => {
                        return Err(Error::new(format!(
                            "badly formatted table list: list names don't \
                             match ('{current}' vs. '{list_name}')"
                        ))
                        .sheet("survey")
                        .row(q.row)
                        .column("type")
                        .hint(
                            "all selects inside a table-list group must share the same choice list",
                        ));
                    }
                    Some(_) => {}
                }
                q.appearance = Some("list-nolabel".into());
                children.push(Item::Question(q));
            }
            other => children.push(other),
        }
    }
    section.children = children;
    Ok(())
}

/// Columns addressed with an explicit `bind::`/`control::`/`body::` prefix
/// (single or double colon). Known sub-columns map back to their canonical
/// survey columns; unknown `bind` sub-columns pass through to the bind
/// element with their original case.
enum PrefixedColumn {
    Canonical(&'static str),
    BindExtra(String),
    BodyExtra(String),
    InstanceAttr(String),
}

/// Recover the original-case tail of `raw` after a case-insensitive prefix.
fn original_tail(raw: &str, rest_lower: &str) -> String {
    let original = raw.trim();
    let idx = original.to_lowercase().find(rest_lower).unwrap_or(0);
    original[idx..].to_string()
}

fn parse_prefixed_column(raw: &str) -> Option<PrefixedColumn> {
    let lower = raw.trim().to_lowercase();
    if let Some(rest) = lower
        .strip_prefix("bind::")
        .or_else(|| lower.strip_prefix("bind:"))
    {
        let canonical = match rest {
            "relevant" => "relevant",
            "constraint" => "constraint",
            "jr:constraintmsg" => "constraint_message",
            "required" => "required",
            "jr:requiredmsg" => "required_message",
            "readonly" => "readonly",
            "calculate" => "calculation",
            // keep the original header's case for passthrough keys
            _ => return Some(PrefixedColumn::BindExtra(original_tail(raw, rest))),
        };
        return Some(PrefixedColumn::Canonical(canonical));
    }
    for prefix in ["instance::", "instance:"] {
        if let Some(rest) = lower.strip_prefix(prefix) {
            return Some(PrefixedColumn::InstanceAttr(original_tail(raw, rest)));
        }
    }
    for prefix in ["control::", "control:", "body::", "body:"] {
        if let Some(rest) = lower.strip_prefix(prefix) {
            let canonical = match rest {
                "appearance" => "appearance",
                "jr:count" => "repeat_count",
                "rows" => "rows",
                "autoplay" => "autoplay",
                // any other control attribute passes straight through
                _ => return Some(PrefixedColumn::BodyExtra(original_tail(raw, rest))),
            };
            return Some(PrefixedColumn::Canonical(canonical));
        }
    }
    None
}

/// Read every non-type column of a survey row into a [`Question`]. The
/// `repeat_count` column is stashed in `bind_extra` for section rows.
fn parse_question_row(
    sheet: &Sheet,
    row: &BTreeMap<String, String>,
    row_number: usize,
) -> Result<Question> {
    let mut q = Question {
        row: row_number,
        ..Question::default()
    };
    for header_raw in &sheet.headers {
        let Some(value) = row.get(header_raw) else {
            continue;
        };
        let base;
        let h;
        match parse_prefixed_column(header_raw) {
            Some(PrefixedColumn::Canonical(c)) => {
                base = c.to_string();
                h = Header {
                    base: base.clone(),
                    lang: None,
                };
            }
            Some(PrefixedColumn::BindExtra(key)) => {
                q.bind_extra.insert(key, value.clone());
                continue;
            }
            Some(PrefixedColumn::BodyExtra(key)) => {
                q.body_attrs.push((key, value.clone()));
                continue;
            }
            Some(PrefixedColumn::InstanceAttr(key)) => {
                q.instance_attrs.push((key, value.clone()));
                continue;
            }
            None => {
                h = parse_header(header_raw);
                base = survey_column_alias(&h.base).to_string();
            }
        }
        match base.as_str() {
            "type" => {}
            "name" => q.name = value.trim().to_string(),
            "label" => set_translated(&mut q.label, &h.lang, value),
            "hint" => set_translated(&mut q.hint, &h.lang, value),
            "guidance_hint" => set_translated(&mut q.guidance_hint, &h.lang, value),
            "constraint_message" => set_translated(&mut q.constraint_message, &h.lang, value),
            "required_message" => set_translated(&mut q.required_message, &h.lang, value),
            m if MEDIA_KINDS.contains(&m) => {
                set_translated(q.media.entry(m.to_string()).or_default(), &h.lang, value)
            }
            "relevant" => q.relevant = Some(value.clone()),
            "constraint" => q.constraint = Some(value.clone()),
            "required" => q.required = Some(binding_value(value)),
            "readonly" => q.readonly = Some(binding_value(value)),
            "calculation" => q.calculation = Some(value.clone()),
            "default" => q.default = Some(value.clone()),
            "trigger" => q.trigger = Some(value.clone()),
            "appearance" => q.appearance = Some(value.clone()),
            "choice_filter" => q.choice_filter = Some(value.clone()),
            "parameters" => {
                let params = parse_parameters(value);
                q.parameters.extend(params);
            }
            "rows" => {
                q.parameters.insert("rows".into(), value.clone());
            }
            "autoplay" => {
                q.parameters.insert("autoplay".into(), value.clone());
            }
            "noapperrorstring" | "no_app_error_string" => {
                q.bind_extra
                    .insert("jr:noAppErrorString".into(), value.clone());
            }
            "compact_tag" => {
                q.instance_attrs.push(("odk:tag".into(), value.clone()));
            }
            "save_to" | "saveto" => {
                q.bind_extra.insert("entities:saveto".into(), value.clone());
            }
            "repeat_count" => {
                q.bind_extra.insert("repeat_count".into(), value.clone());
            }
            b if b.starts_with("bind:") => {
                let key = b.trim_start_matches("bind:").trim_start_matches(':');
                q.bind_extra.insert(key.to_string(), value.clone());
            }
            _ => {} // unknown columns are ignored, like pyxform's defaults
        }
    }
    let _ = row_number;
    Ok(q)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xls::sheet_from_rows;

    fn s(rows: &[&[&str]]) -> Sheet {
        sheet_from_rows(
            rows.iter()
                .map(|r| r.iter().map(|c| c.to_string()).collect()),
        )
    }

    #[test]
    fn parses_select_types() {
        match parse_type_cell("select_one yes_no") {
            Some(RowType::Question { qtype, select, .. }) => {
                assert_eq!(qtype, "select one");
                assert_eq!(select, Some((SelectKind::One, "yes_no".into())));
            }
            _ => panic!("expected select one"),
        }
        match parse_type_cell("select multiple  colors") {
            Some(RowType::Question { select, .. }) => {
                assert_eq!(select, Some((SelectKind::Multiple, "colors".into())));
            }
            _ => panic!("expected select multiple"),
        }
        assert!(parse_type_cell("no_such_type").is_none());
    }

    #[test]
    fn builds_nested_sections() {
        let sheet = s(&[
            &["type", "name", "label"],
            &["text", "a", "A"],
            &["begin group", "g", "G"],
            &["integer", "b", "B"],
            &["begin repeat", "r", "R"],
            &["text", "c", "C"],
            &["end repeat", "", ""],
            &["end group", "", ""],
        ]);
        let items = parse_survey(&sheet).unwrap();
        assert_eq!(items.len(), 2);
        let Item::Section(g) = &items[1] else {
            panic!("expected group")
        };
        assert_eq!(g.kind, SectionKind::Group);
        assert_eq!(g.children.len(), 2);
        let Item::Section(r) = &g.children[1] else {
            panic!("expected repeat")
        };
        assert_eq!(r.kind, SectionKind::Repeat);
    }

    #[test]
    fn mismatched_end_errors() {
        let sheet = s(&[
            &["type", "name"],
            &["begin group", "g"],
            &["end repeat", ""],
        ]);
        assert!(parse_survey(&sheet).is_err());
    }

    #[test]
    fn parses_languages_and_media() {
        let sheet = s(&[
            &[
                "type",
                "name",
                "label::English (en)",
                "label::Português (pt)",
                "media::image",
            ],
            &["text", "q1", "Name?", "Nome?", "pic.jpg"],
        ]);
        let items = parse_survey(&sheet).unwrap();
        let Item::Question(q) = &items[0] else {
            panic!()
        };
        assert_eq!(q.label.get("English (en)").unwrap(), "Name?");
        assert_eq!(q.label.get("Português (pt)").unwrap(), "Nome?");
        assert_eq!(q.media["image"][DEFAULT_LANG], "pic.jpg");
    }

    #[test]
    fn parses_choices_with_extras() {
        let sheet = s(&[
            &["list_name", "name", "label", "state"],
            &["cities", "rec", "Recife", "PE"],
            &["cities", "poa", "Porto Alegre", "RS"],
            &["yn", "yes", "Yes", ""],
        ]);
        let lists = parse_choices(&sheet).unwrap();
        assert_eq!(lists.len(), 2);
        assert_eq!(lists[0].name, "cities");
        assert_eq!(
            lists[0].choices[0].extras,
            vec![("state".to_string(), "PE".to_string())]
        );
    }
}
