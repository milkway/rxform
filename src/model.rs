//! In-memory representation of a parsed XLSForm.

use std::collections::BTreeMap;

/// The language key used for untagged columns (plain `label` vs `label::Lang`).
pub const DEFAULT_LANG: &str = "default";

/// A translatable value: language name → text. Untagged columns are stored
/// under [`DEFAULT_LANG`].
pub type Translated = BTreeMap<String, String>;

#[derive(Debug, Default, Clone)]
pub struct Settings {
    pub form_title: Option<String>,
    pub form_id: Option<String>,
    /// Root node name of the primary instance (settings `name` column).
    pub name: Option<String>,
    pub version: Option<String>,
    pub default_language: Option<String>,
    pub instance_name: Option<String>,
    pub style: Option<String>,
    pub submission_url: Option<String>,
    pub public_key: Option<String>,
    pub auto_send: Option<String>,
    pub auto_delete: Option<String>,
    /// Choice lists may repeat a name when this is set (settings column
    /// `allow_choice_duplicates`).
    pub allow_choice_duplicates: bool,
    /// Extra namespaces for the root element: `key=uri` pairs, space-separated.
    pub namespaces: Option<String>,
    /// `attribute::x` settings columns → attributes on the primary instance root.
    pub attributes: Vec<(String, String)>,
    /// `flat=yes`: groups keep their body element but vanish from the
    /// instance tree and from xpaths.
    pub flat: bool,
    /// `omit_instanceID=yes`: no meta/instanceID node or bind.
    pub omit_instance_id: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectKind {
    One,
    Multiple,
    Rank,
}

#[derive(Debug, Default, Clone)]
pub struct Question {
    /// Canonical type name after alias resolution (e.g. "text", "select one").
    pub qtype: String,
    pub name: String,
    pub label: Translated,
    pub hint: Translated,
    pub guidance_hint: Translated,
    /// media kind ("image", "audio", "video", "big-image") → translated URI.
    pub media: BTreeMap<String, Translated>,
    pub select: Option<(SelectKind, String)>,
    pub choice_filter: Option<String>,
    pub relevant: Option<String>,
    pub constraint: Option<String>,
    pub constraint_message: Translated,
    pub required: Option<String>,
    pub required_message: Translated,
    pub calculation: Option<String>,
    pub default: Option<String>,
    pub trigger: Option<String>,
    pub readonly: Option<String>,
    pub appearance: Option<String>,
    /// Raw `parameters` column, e.g. "start=0 end=10 step=2".
    pub parameters: BTreeMap<String, String>,
    /// Extra `bind::*` columns passed straight through to the bind element.
    pub bind_extra: BTreeMap<String, String>,
    /// 1-based spreadsheet row this question came from.
    pub row: usize,
    /// `select_* ... or_other` was used.
    pub or_other: bool,
    /// `instance::attr` columns → attributes on this question's instance node.
    pub instance_attrs: Vec<(String, String)>,
    /// Unrecognized `body::attr` columns → attributes on the body control.
    pub body_attrs: Vec<(String, String)>,
}

/// File extensions accepted for `select_*_from_file` and external instances.
pub const EXTERNAL_INSTANCE_EXTENSIONS: [&str; 3] = ["csv", "xml", "geojson"];

impl Question {
    /// For `select_one_from_file fruits.csv`, returns `("fruits", "csv")`.
    pub fn select_from_file(&self) -> Option<(&str, &str)> {
        let (_, list) = self.select.as_ref()?;
        let (base, ext) = list.rsplit_once('.')?;
        if EXTERNAL_INSTANCE_EXTENSIONS.contains(&ext) {
            Some((base, ext))
        } else {
            None
        }
    }

    /// Approximation of pyxform's `default_is_dynamic`: expressions (with
    /// references, function calls, predicates or arithmetic) become
    /// `odk:setvalue` actions instead of static instance values.
    pub fn default_is_dynamic(&self) -> bool {
        let Some(default) = &self.default else {
            return false;
        };
        if default.contains("${") || default.contains('(') || default.contains('[') {
            return true;
        }
        let date_like = matches!(
            self.qtype.as_str(),
            "date" | "datetime" | "dateTime" | "date time" | "geopoint" | "geotrace" | "geoshape"
        );
        let mut ops: Vec<char> = vec!['+', '*', '|'];
        if !date_like {
            ops.push('-');
        }
        default.contains(&ops[..]) || default.split_whitespace().any(|t| t == "div" || t == "mod")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SectionKind {
    Group,
    Repeat,
    /// `begin loop over <list>`: expanded by the parser into one sub-group
    /// per choice; never reaches XML generation.
    Loop,
}

#[derive(Debug, Clone)]
pub struct Section {
    pub kind: SectionKind,
    pub name: String,
    pub label: Translated,
    pub hint: Translated,
    pub relevant: Option<String>,
    pub appearance: Option<String>,
    /// `repeat_count` column (repeats only).
    pub count: Option<String>,
    pub children: Vec<Item>,
    /// 1-based spreadsheet row of the `begin group/repeat` line.
    pub row: usize,
    /// The choice list a `begin loop` iterates over.
    pub loop_list: Option<String>,
    /// Groups under `settings flat=yes`: body only, no instance node.
    pub flat: bool,
}

#[derive(Debug, Clone)]
pub enum Item {
    Question(Box<Question>),
    Section(Section),
}

#[derive(Debug, Clone)]
pub struct Choice {
    pub name: String,
    pub label: Translated,
    pub media: BTreeMap<String, Translated>,
    /// Extra columns (used by choice_filter), in sheet column order.
    pub extras: Vec<(String, String)>,
    /// 1-based spreadsheet row this choice came from (0 for generated ones).
    pub row: usize,
}

#[derive(Debug, Clone)]
pub struct ChoiceList {
    pub name: String,
    pub choices: Vec<Choice>,
}

impl ChoiceList {
    /// True if any choice needs itext (a non-default language or any media).
    pub fn needs_itext(&self) -> bool {
        self.choices
            .iter()
            .any(|c| !c.media.is_empty() || c.label.keys().any(|lang| lang != DEFAULT_LANG))
    }
}

/// The `external_choices` sheet, kept verbatim: it is not part of the XForm
/// XML but is exported as `itemsets.csv` for `select_one_external` clients.
#[derive(Debug, Clone, Default)]
pub struct ExternalChoices {
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

/// A row of the `entities` sheet: this form creates and/or updates an
/// entity in `dataset`.
#[derive(Debug, Clone)]
pub struct EntityDecl {
    pub dataset: String,
    pub entity_id: Option<String>,
    pub create_if: Option<String>,
    pub update_if: Option<String>,
    pub label: Option<String>,
    pub row: usize,
}

impl EntityDecl {
    /// The form updates an existing entity (an entity_id is given).
    pub fn updates(&self) -> bool {
        self.entity_id.is_some()
    }

    /// The form (conditionally) creates an entity.
    pub fn creates(&self) -> bool {
        self.entity_id.is_none() || self.create_if.is_some()
    }
}

#[derive(Debug, Clone)]
pub struct Survey {
    /// Root element name of the primary instance.
    pub name: String,
    pub id_string: String,
    pub title: String,
    pub settings: Settings,
    pub items: Vec<Item>,
    /// Choice lists in order of first appearance in the choices sheet.
    pub choice_lists: Vec<ChoiceList>,
    pub external_choices: Option<ExternalChoices>,
    pub entity: Option<EntityDecl>,
}

impl Survey {
    pub fn choice_list(&self, name: &str) -> Option<&ChoiceList> {
        self.choice_lists.iter().find(|l| l.name == name)
    }

    /// The xpath prefix a section's children live under: flat groups
    /// disappear from paths.
    pub fn child_prefix(section: &Section, prefix: &str) -> String {
        if section.flat {
            prefix.to_string()
        } else {
            format!("{prefix}/{}", section.name)
        }
    }

    /// Depth-first walk over questions with their absolute xpaths.
    pub fn walk(&self) -> Vec<(String, &Question)> {
        fn rec<'a>(items: &'a [Item], prefix: &str, out: &mut Vec<(String, &'a Question)>) {
            for item in items {
                match item {
                    Item::Question(q) => out.push((format!("{prefix}/{}", q.name), q)),
                    Item::Section(s) => rec(&s.children, &Survey::child_prefix(s, prefix), out),
                }
            }
        }
        let mut out = Vec::new();
        rec(&self.items, &format!("/{}", self.name), &mut out);
        out
    }

    /// Every xpath each name resolves to. A name with more than one entry is
    /// ambiguous and cannot be used in `${references}`.
    pub fn xpath_multi_index(&self) -> BTreeMap<String, Vec<String>> {
        fn rec(items: &[Item], prefix: &str, out: &mut BTreeMap<String, Vec<String>>) {
            for item in items {
                match item {
                    Item::Question(q) => {
                        out.entry(q.name.clone())
                            .or_default()
                            .push(format!("{prefix}/{}", q.name));
                    }
                    Item::Section(s) => {
                        let path = Survey::child_prefix(s, prefix);
                        out.entry(s.name.clone()).or_default().push(path.clone());
                        rec(&s.children, &path, out);
                    }
                }
            }
        }
        let mut out = BTreeMap::new();
        rec(&self.items, &format!("/{}", self.name), &mut out);
        out
    }

    /// Map of question/section name → absolute xpath, for `${ref}` expansion.
    /// For ambiguous names this keeps the first occurrence; use
    /// [`Survey::xpath_multi_index`] to detect ambiguity.
    pub fn xpath_index(&self) -> BTreeMap<String, String> {
        self.xpath_multi_index()
            .into_iter()
            .map(|(name, mut paths)| (name, paths.swap_remove(0)))
            .collect()
    }

    /// All languages used anywhere in the form, default language first.
    pub fn languages(&self) -> Vec<String> {
        let mut langs = std::collections::BTreeSet::new();
        fn collect_tr(tr: &Translated, langs: &mut std::collections::BTreeSet<String>) {
            for lang in tr.keys() {
                langs.insert(lang.clone());
            }
        }
        fn rec(items: &[Item], langs: &mut std::collections::BTreeSet<String>) {
            for item in items {
                match item {
                    Item::Question(q) => {
                        collect_tr(&q.label, langs);
                        collect_tr(&q.hint, langs);
                        collect_tr(&q.guidance_hint, langs);
                        collect_tr(&q.constraint_message, langs);
                        collect_tr(&q.required_message, langs);
                        for tr in q.media.values() {
                            collect_tr(tr, langs);
                        }
                    }
                    Item::Section(s) => {
                        collect_tr(&s.label, langs);
                        rec(&s.children, langs);
                    }
                }
            }
        }
        rec(&self.items, &mut langs);
        for list in &self.choice_lists {
            for c in &list.choices {
                collect_tr(&c.label, &mut langs);
                for tr in c.media.values() {
                    collect_tr(tr, &mut langs);
                }
            }
        }
        let default = self
            .settings
            .default_language
            .clone()
            .unwrap_or_else(|| DEFAULT_LANG.to_string());
        let mut out: Vec<String> = Vec::new();
        if langs.contains(&default) || langs.is_empty() {
            out.push(default.clone());
        }
        for lang in langs {
            if lang != default {
                out.push(lang);
            }
        }
        if out.is_empty() {
            out.push(DEFAULT_LANG.to_string());
        }
        out
    }

    /// Whether output must route translatable text through `<itext>`:
    /// true when any language other than the default-untagged one is used,
    /// or any media is attached to labels/choices.
    pub fn needs_itext(&self) -> bool {
        let langs = self.languages();
        if langs.len() > 1 || langs[0] != DEFAULT_LANG {
            return true;
        }
        fn has_media(items: &[Item]) -> bool {
            items.iter().any(|item| match item {
                Item::Question(q) => !q.media.is_empty(),
                Item::Section(s) => has_media(&s.children),
            })
        }
        has_media(&self.items) || self.choice_lists.iter().any(|l| l.needs_itext())
    }
}
