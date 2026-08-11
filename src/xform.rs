//! Generates ODK XForm XML from a parsed [`Survey`], following pyxform's
//! output conventions (element order, itext ids, `${ref}` expansion).

use std::collections::{BTreeMap, BTreeSet};

use crate::error::{Error, Result};
use crate::model::*;
use crate::types::{self, TypeDef};
use crate::xmlwriter::{escape_text, XmlNode};

/// Any `${last-saved#...}` reference anywhere in the form's expressions.
fn survey_uses_last_saved(survey: &Survey) -> bool {
    fn tr_has(tr: &Translated) -> bool {
        tr.values().any(|v| v.contains("${last-saved#"))
    }
    fn rec(items: &[Item]) -> bool {
        items.iter().any(|item| match item {
            Item::Question(q) => {
                [
                    &q.relevant,
                    &q.constraint,
                    &q.calculation,
                    &q.required,
                    &q.choice_filter,
                    &q.default,
                    &q.appearance,
                ]
                .into_iter()
                .flatten()
                .any(|v| v.contains("${last-saved#"))
                    || q.bind_extra.values().any(|v| v.contains("${last-saved#"))
                    || tr_has(&q.label)
                    || tr_has(&q.hint)
            }
            Item::Section(s) => {
                s.relevant
                    .iter()
                    .chain(s.count.iter())
                    .any(|v| v.contains("${last-saved#"))
                    || rec(&s.children)
            }
        })
    }
    rec(&survey.items)
        || survey
            .settings
            .instance_name
            .as_deref()
            .is_some_and(|v| v.contains("${last-saved#"))
}

pub fn generate(survey: &Survey) -> Result<String> {
    let ctx = Context::new(survey)?;

    let mut model = XmlNode::new("model").attr("odk:xforms-version", "1.0.0");
    if survey.entity.is_some() {
        model = model.attr("entities:entities-version", "2024.1.0");
    }
    if let Some(submission) = build_submission(&survey.settings) {
        model.push(submission);
    }
    if let Some(itext) = ctx.build_itext(survey) {
        model.push(itext);
    }
    model.push(XmlNode::new("instance").child(ctx.build_primary_instance(survey)?));
    for instance in ctx.build_secondary_instances(survey)? {
        model.push(instance);
    }
    for bind in ctx.build_binds(survey)? {
        model.push(bind);
    }

    let head = XmlNode::new("h:head")
        .child(XmlNode::new("h:title").text(&survey.title))
        .child(model);

    let mut body = XmlNode::new("h:body");
    if let Some(style) = &survey.settings.style {
        body = body.attr("class", style);
    }
    for item in &survey.items {
        if let Some(control) = ctx.build_control(survey, item, &format!("/{}", survey.name))? {
            body.push(control);
        }
    }

    let mut html = XmlNode::new("h:html")
        .attr("xmlns", "http://www.w3.org/2002/xforms")
        .attr("xmlns:ev", "http://www.w3.org/2001/xml-events")
        .attr("xmlns:h", "http://www.w3.org/1999/xhtml")
        .attr("xmlns:jr", "http://openrosa.org/javarosa")
        .attr("xmlns:odk", "http://www.opendatakit.org/xforms")
        .attr("xmlns:orx", "http://openrosa.org/xforms")
        .attr("xmlns:xsd", "http://www.w3.org/2001/XMLSchema");
    if survey.entity.is_some() {
        html = html.attr(
            "xmlns:entities",
            "http://www.opendatakit.org/xforms/entities",
        );
    }
    // settings `namespaces`: space-separated key=uri pairs; the standard
    // prefixes above stay authoritative
    if let Some(namespaces) = &survey.settings.namespaces {
        const STANDARD: [&str; 6] = ["ev", "h", "jr", "odk", "orx", "xsd"];
        for pair in namespaces.split_whitespace() {
            if let Some((key, uri)) = pair.split_once('=') {
                if !key.is_empty() && !STANDARD.contains(&key) {
                    html = html.attr(&format!("xmlns:{key}"), uri.trim_matches(['"', '\'']));
                }
            }
        }
    }
    let html = html.child(head).child(body);

    Ok(html.to_document())
}

/// The `itemsets.csv` companion file for `select_one_external`: the
/// external_choices sheet re-encoded as a quoted CSV.
pub fn itemsets_csv(survey: &Survey) -> Option<String> {
    let external = survey.external_choices.as_ref()?;
    let quote = |cell: &str| format!("\"{}\"", cell.replace('"', "\"\""));
    // \r\n line endings, matching Python's csv module (what pyxform uses)
    let mut out = String::new();
    out.push_str(
        &external
            .headers
            .iter()
            .map(|h| quote(h))
            .collect::<Vec<_>>()
            .join(","),
    );
    out.push_str("\r\n");
    for row in &external.rows {
        out.push_str(&row.iter().map(|c| quote(c)).collect::<Vec<_>>().join(","));
        out.push_str("\r\n");
    }
    Some(out)
}

/// Precomputed generation state: xpath index, language list, and which
/// element/content pairs are routed through `<itext>`.
struct Context {
    xpaths: BTreeMap<String, String>,
    languages: Vec<String>,
    default_language: String,
    /// itext ids that exist, e.g. `/form/q1:label`.
    itext_ids: BTreeSet<String>,
    /// trigger-source question name → actions fired when its value changes.
    triggers: BTreeMap<String, Vec<TriggerAction>>,
    /// Names used by more than one field: banned in `${references}`.
    ambiguous: BTreeSet<String>,
    /// Choice lists consumed by `search()` selects: inline items, no instance.
    searched_lists: BTreeSet<String>,
    /// Any `${last-saved#field}` reference exists in the form.
    uses_last_saved: bool,
}

/// A `<setvalue>`/`<odk:setgeopoint>` fired from inside a body control when
/// the hosting question's value changes.
struct TriggerAction {
    /// Absolute xpath of the question being written to.
    target: String,
    /// Expanded value expression; `None` clears the target (or, for
    /// geopoint, captures the location).
    value: Option<String>,
    geopoint: bool,
}

impl Context {
    fn new(survey: &Survey) -> Result<Self> {
        // The declared default language ("default" unless the settings sheet
        // says otherwise). May be absent from `languages` when every
        // translatable column is language-tagged.
        let default_language = survey
            .settings
            .default_language
            .clone()
            .unwrap_or_else(|| DEFAULT_LANG.to_string());
        let ambiguous: BTreeSet<String> = survey
            .xpath_multi_index()
            .into_iter()
            .filter(|(_, paths)| paths.len() > 1)
            .map(|(name, _)| name)
            .collect();
        let searched_lists: BTreeSet<String> = survey
            .walk()
            .iter()
            .filter_map(|(_, q)| {
                let (_, list) = q.select.as_ref()?;
                let appearance = q.appearance.as_deref()?;
                if appearance.to_lowercase().contains("search(") {
                    Some(list.clone())
                } else {
                    None
                }
            })
            .collect();
        let mut ctx = Context {
            xpaths: survey.xpath_index(),
            languages: Vec::new(),
            default_language,
            itext_ids: BTreeSet::new(),
            triggers: BTreeMap::new(),
            ambiguous,
            searched_lists,
            uses_last_saved: survey_uses_last_saved(survey),
        };

        // Wire up trigger-column actions: when the source question changes,
        // set (or clear) the target, or capture a background geopoint.
        let mut triggers: BTreeMap<String, Vec<TriggerAction>> = BTreeMap::new();
        for (xpath, q) in survey.walk() {
            let Some(trigger) = &q.trigger else { continue };
            let source = trigger
                .trim()
                .trim_start_matches("${")
                .trim_end_matches('}')
                .trim()
                .to_string();
            let geopoint = q.qtype == "background-geopoint";
            let value = match (&q.calculation, geopoint) {
                (Some(calc), false) => Some(ctx.expand(calc)?),
                _ => None,
            };
            triggers.entry(source).or_default().push(TriggerAction {
                target: xpath,
                value,
                geopoint,
            });
        }
        ctx.triggers = triggers;

        // A piece of content is routed through itext only when it needs it:
        // it has language-tagged values, attached media, or a guidance hint.
        for (xpath, element) in document_order(&survey.items, &format!("/{}", survey.name)) {
            let (label, hint, guidance, media, cmsg, rmsg) = element.translatables();
            if has_translation(label) || !media.is_empty() {
                ctx.itext_ids.insert(format!("{xpath}:label"));
            }
            if has_translation(hint) || !guidance.is_empty() {
                ctx.itext_ids.insert(format!("{xpath}:hint"));
            }
            if has_translation(cmsg) {
                ctx.itext_ids.insert(format!("{xpath}:jr:constraintMsg"));
            }
            if has_translation(rmsg) {
                ctx.itext_ids.insert(format!("{xpath}:jr:requiredMsg"));
            }
        }

        // The translation languages are those of content that actually goes
        // through itext; a plain-string label on a non-itext element does
        // not create a "default" translation (pyxform's behavior).
        let mut langs: BTreeSet<String> = BTreeSet::new();
        let mut collect = |tr: &Translated| {
            for lang in tr.keys() {
                langs.insert(lang.clone());
            }
        };
        for (xpath, element) in document_order(&survey.items, &format!("/{}", survey.name)) {
            let (label, hint, guidance, media, cmsg, rmsg) = element.translatables();
            if ctx.itext_ids.contains(&format!("{xpath}:label")) {
                collect(label);
                for tr in media.values() {
                    collect(tr);
                }
            }
            if ctx.itext_ids.contains(&format!("{xpath}:hint")) {
                collect(hint);
                collect(guidance);
            }
            if ctx.itext_ids.contains(&format!("{xpath}:jr:constraintMsg")) {
                collect(cmsg);
            }
            if ctx.itext_ids.contains(&format!("{xpath}:jr:requiredMsg")) {
                collect(rmsg);
            }
        }
        for list in &survey.choice_lists {
            if list.needs_itext() {
                for choice in &list.choices {
                    collect(&choice.label);
                    for tr in choice.media.values() {
                        collect(tr);
                    }
                }
            }
        }
        let mut languages: Vec<String> = Vec::new();
        if langs.contains(&ctx.default_language) || langs.is_empty() {
            languages.push(ctx.default_language.clone());
        }
        for lang in langs {
            if lang != ctx.default_language {
                languages.push(lang);
            }
        }
        if languages.is_empty() {
            languages.push(DEFAULT_LANG.to_string());
        }
        ctx.languages = languages;
        Ok(ctx)
    }

    /// Look up a translated value the way pyxform does: an untagged column
    /// value counts as the default language; anything else missing is "-".
    fn tr<'a>(&self, map: &'a Translated, lang: &str) -> Option<&'a str> {
        map.get(lang)
            .or_else(|| {
                if lang == self.default_language {
                    map.get(DEFAULT_LANG)
                } else {
                    None
                }
            })
            .map(String::as_str)
    }

    fn tr_or_dash<'a>(&self, map: &'a Translated, lang: &str) -> &'a str {
        self.tr(map, lang).unwrap_or("-")
    }

    /// Expand `${name}` references inside an XPath expression. Matches
    /// pyxform, which pads the absolute path with spaces.
    fn expand(&self, expr: &str) -> Result<String> {
        self.expand_with(expr, |path, out| {
            out.push(' ');
            out.push_str(path);
            out.push(' ');
        })
    }

    /// Expand `${name}` in label/hint text into `<output/>` elements,
    /// escaping everything else. Returns (fragment, contains_output).
    /// Mixed content is padded with spaces, matching pyxform's printer.
    fn expand_label(&self, text: &str) -> Result<(String, bool)> {
        let mut has_output = false;
        let out = self.expand_with_escaped(text, |path, out| {
            has_output = true;
            out.push_str("<output value=\" ");
            out.push_str(path);
            out.push_str(" \"/>");
        })?;
        if has_output {
            return Ok((format!(" {out} "), true));
        }
        Ok((out, has_output))
    }

    fn expand_with(&self, expr: &str, mut emit: impl FnMut(&str, &mut String)) -> Result<String> {
        let mut out = String::new();
        let mut rest = expr;
        while let Some(start) = rest.find("${") {
            out.push_str(&rest[..start]);
            let after = &rest[start + 2..];
            let end = after
                .find('}')
                .ok_or_else(|| Error::new(format!("unterminated ${{...}} in '{expr}'")))?;
            let name = after[..end].trim();
            let (lookup, instance_prefix) = match name.strip_prefix("last-saved#") {
                Some(field) => (field.trim(), "instance('__last-saved')"),
                None => (name, ""),
            };
            if self.ambiguous.contains(lookup) {
                return Err(Error::new(format!(
                    "'${{{lookup}}}' is ambiguous: more than one field has this name"
                )));
            }
            let path = self.xpaths.get(lookup).ok_or_else(|| {
                Error::new(format!("'${{{name}}}' does not match any field name"))
            })?;
            let prefixed = format!("{instance_prefix}{path}");
            emit(&prefixed, &mut out);
            rest = &after[end + 1..];
        }
        out.push_str(rest);
        Ok(out)
    }

    fn expand_with_escaped(
        &self,
        expr: &str,
        mut emit: impl FnMut(&str, &mut String),
    ) -> Result<String> {
        let mut out = String::new();
        let mut rest = expr;
        while let Some(start) = rest.find("${") {
            out.push_str(&escape_text(&rest[..start]));
            let after = &rest[start + 2..];
            let end = after
                .find('}')
                .ok_or_else(|| Error::new(format!("unterminated ${{...}} in '{expr}'")))?;
            let name = after[..end].trim();
            let (lookup, instance_prefix) = match name.strip_prefix("last-saved#") {
                Some(field) => (field.trim(), "instance('__last-saved')"),
                None => (name, ""),
            };
            if self.ambiguous.contains(lookup) {
                return Err(Error::new(format!(
                    "'${{{lookup}}}' is ambiguous: more than one field has this name"
                )));
            }
            let path = self.xpaths.get(lookup).ok_or_else(|| {
                Error::new(format!("'${{{name}}}' does not match any field name"))
            })?;
            let prefixed = format!("{instance_prefix}{path}");
            emit(&prefixed, &mut out);
            rest = &after[end + 1..];
        }
        out.push_str(&escape_text(rest));
        Ok(out)
    }

    // -- itext ---------------------------------------------------------------

    fn build_itext(&self, survey: &Survey) -> Option<XmlNode> {
        let mut translations: Vec<XmlNode> = Vec::new();
        for lang in &self.languages {
            let mut translation = XmlNode::new("translation").attr("lang", lang);
            // pyxform only marks a translation as default when it matches the
            // declared default language (settings, or the untagged "default").
            if lang == &self.default_language {
                translation = translation.attr("default", "true()");
            }
            let mut entries: Vec<(XmlNode, u8)> = Vec::new();
            // Choice-list texts come first, matching pyxform's itext order.
            for list in &survey.choice_lists {
                if !list.needs_itext() {
                    continue;
                }
                for (i, choice) in list.choices.iter().enumerate() {
                    let text = XmlNode::new("text").attr("id", &format!("{}-{}", list.name, i));
                    let (node, priority) =
                        self.fill_label_text(text, &choice.label, &choice.media, lang);
                    entries.push((node, priority));
                }
            }
            for (xpath, element) in document_order(&survey.items, &format!("/{}", survey.name)) {
                let (label, hint, guidance, media, cmsg, rmsg) = element.translatables();
                if self.itext_ids.contains(&format!("{xpath}:label")) {
                    let text = XmlNode::new("text").attr("id", &format!("{xpath}:label"));
                    let (node, priority) = self.fill_label_text(text, label, media, lang);
                    entries.push((node, priority));
                }
                if self.itext_ids.contains(&format!("{xpath}:hint")) {
                    let mut text = XmlNode::new("text").attr("id", &format!("{xpath}:hint"));
                    let mut priority = 0;
                    match self.tr(hint, lang) {
                        Some(real) => {
                            priority = 2;
                            text.push(self.label_value_node(real));
                            if let Some(g) = self.tr(guidance, lang) {
                                text.push(XmlNode::new("value").attr("form", "guidance").text(g));
                            }
                        }
                        None => {
                            if let Some(g) = self.tr(guidance, lang) {
                                priority = 2;
                                text.push(XmlNode::new("value").attr("form", "guidance").text(g));
                            }
                            if !hint.is_empty() {
                                priority = priority.max(1);
                                text.push(XmlNode::new("value").text("-"));
                            }
                        }
                    }
                    entries.push((text, priority));
                }
                for (key, map) in [("jr:constraintMsg", cmsg), ("jr:requiredMsg", rmsg)] {
                    if self.itext_ids.contains(&format!("{xpath}:{key}")) {
                        let priority = if self.tr(map, lang).is_some() { 2 } else { 1 };
                        entries.push((
                            XmlNode::new("text")
                                .attr("id", &format!("{xpath}:{key}"))
                                .child(self.label_value_node(self.tr_or_dash(map, lang))),
                            priority,
                        ));
                    }
                }
            }
            // Duplicate text ids (e.g. several flat groups labelled at the
            // same xpath) collapse dict-style: first position wins for
            // placement; the last entry with real content wins for value.
            let mut by_id: BTreeMap<String, usize> = BTreeMap::new();
            let mut deduped: Vec<(XmlNode, u8)> = Vec::new();
            for (text, priority) in entries {
                let id = text
                    .attrs
                    .iter()
                    .find(|(k, _)| k == "id")
                    .map(|(_, v)| v.clone())
                    .unwrap_or_default();
                match by_id.get(&id) {
                    Some(&index) => {
                        if priority >= deduped[index].1 && priority == 2
                            || priority > deduped[index].1
                        {
                            deduped[index] = (text, priority);
                        }
                    }
                    None => {
                        by_id.insert(id, deduped.len());
                        deduped.push((text, priority));
                    }
                }
            }
            for (text, _) in deduped {
                translation.push(text);
            }
            translations.push(translation);
        }
        if translations.iter().all(|t| t.children.is_empty()) {
            return None;
        }
        let mut itext = XmlNode::new("itext");
        for t in translations {
            itext.push(t);
        }
        Some(itext)
    }

    /// Fill an itext `<text>` node for a label: pyxform emits the real text
    /// value first when this language has one; a "-" placeholder (only added
    /// when the label has text in some language) goes after the media forms.
    /// Also returns a merge priority for colliding itext ids (flat groups
    /// can share one): 2 = real content for this language, 1 = "-" filler,
    /// 0 = empty node.
    fn fill_label_text(
        &self,
        mut text: XmlNode,
        label: &Translated,
        media: &BTreeMap<String, Translated>,
        lang: &str,
    ) -> (XmlNode, u8) {
        let real = self.tr(label, lang);
        let mut had_media = false;
        if let Some(real) = real {
            text.push(self.label_value_node(real));
        }
        for (kind, uris) in ordered_media(media) {
            if let Some(uri) = self.tr(uris, lang) {
                had_media = true;
                text.push(
                    XmlNode::new("value")
                        .attr("form", kind)
                        .text(&media_uri(kind, uri)),
                );
            }
        }
        // the "-" text filler applies whenever the label has text in some
        // language but not this one — even alongside media values
        if real.is_none() && !label.is_empty() {
            text.push(XmlNode::new("value").text("-"));
        }
        let priority = if real.is_some() || had_media {
            2
        } else if !label.is_empty() {
            1
        } else {
            0
        };
        (text, priority)
    }

    fn label_value_node(&self, value: &str) -> XmlNode {
        match self.expand_label(value) {
            Ok((fragment, true)) => XmlNode::new("value").raw_text(&fragment),
            _ => XmlNode::new("value").text(value),
        }
    }

    // -- primary instance ----------------------------------------------------

    fn build_primary_instance(&self, survey: &Survey) -> Result<XmlNode> {
        let mut root = XmlNode::new(&survey.name).attr("id", &survey.id_string);
        // settings `attribute::x` columns become root attributes; id/version
        // stay authoritative
        for (key, value) in &survey.settings.attributes {
            if key != "id" && key != "version" {
                root = root.attr(key, value);
            }
        }
        if let Some(version) = &survey.settings.version {
            root = root.attr("version", version);
        }
        for item in &survey.items {
            self.push_instance_nodes(&mut root, item)?;
        }
        let mut meta = XmlNode::new("meta");
        if let Some(entity) = &survey.entity {
            let mut node = XmlNode::new("entity").attr("dataset", &entity.dataset);
            if entity.creates() {
                node = node.attr("create", "1");
            }
            if entity.updates() {
                node = node
                    .attr("update", "1")
                    .attr("baseVersion", "")
                    .attr("trunkVersion", "")
                    .attr("branchId", "");
            }
            node = node.attr("id", "");
            if entity.label.is_some() {
                node.push(XmlNode::new("label"));
            }
            meta.push(node);
        }
        // audit lives inside meta, ahead of instanceID (pyxform's layout)
        for (_, q) in survey.walk() {
            if q.qtype == "audit" {
                meta.push(XmlNode::new(&q.name));
            }
        }
        if !survey.settings.omit_instance_id {
            meta.push(XmlNode::new("instanceID"));
        }
        if survey.settings.instance_name.is_some() {
            meta.push(XmlNode::new("instanceName"));
        }
        if !meta.children.is_empty() {
            root.push(meta);
        }
        Ok(root)
    }

    fn push_instance_nodes(&self, parent: &mut XmlNode, item: &Item) -> Result<()> {
        match item {
            Item::Question(q) => {
                // external data rows only produce an <instance src=...>;
                // audit is emitted inside <meta> instead
                if matches!(q.qtype.as_str(), "xml-external" | "csv-external" | "audit") {
                    return Ok(());
                }
                let mut node = XmlNode::new(&q.name);
                for (key, value) in &q.instance_attrs {
                    node = node.attr(key, &self.expand(value)?);
                }
                // dynamic defaults are set via odk:setvalue actions instead
                if let (Some(default), false) = (&q.default, q.default_is_dynamic()) {
                    node = node.text(default);
                }
                parent.push(node);
            }
            Item::Section(s) => {
                if s.flat {
                    for child in &s.children {
                        self.push_instance_nodes(parent, child)?;
                    }
                    return Ok(());
                }
                if s.kind == SectionKind::Repeat {
                    // pyxform writes a jr:template copy followed by a
                    // regular first instance of the repeat.
                    let mut template = XmlNode::new(&s.name).attr("jr:template", "");
                    for child in &s.children {
                        self.push_instance_nodes(&mut template, child)?;
                    }
                    parent.push(template);
                }
                let mut node = XmlNode::new(&s.name);
                for child in &s.children {
                    self.push_instance_nodes(&mut node, child)?;
                }
                parent.push(node);
            }
        }
        Ok(())
    }

    // -- secondary instances -------------------------------------------------

    /// External-data instances (pulldata csv, from-file selects, xml/csv
    /// external rows), in document order, then the choice-list instances.
    /// Duplicate (id, src) pairs collapse; the same id with a different src
    /// is an error; a choice list whose name clashes with an external id is
    /// dropped (pyxform's rules).
    fn build_secondary_instances(&self, survey: &Survey) -> Result<Vec<XmlNode>> {
        let mut out = Vec::new();
        let mut seen: BTreeMap<String, String> = BTreeMap::new();
        for (_, q) in survey.walk() {
            for (id, src) in external_instances_for(q) {
                match seen.get(&id) {
                    Some(existing) if *existing == src => {}
                    Some(existing) => {
                        return Err(Error::new(format!(
                            "two external data sources use the instance id '{id}' \
                             with different files: '{existing}' vs '{src}'"
                        ))
                        .sheet("survey")
                        .row(q.row)
                        .hint("rename one of the files so each data source has a unique name"));
                    }
                    None => {
                        seen.insert(id.clone(), src.clone());
                        out.push(XmlNode::new("instance").attr("id", &id).attr("src", &src));
                    }
                }
            }
        }
        if self.uses_last_saved {
            out.push(
                XmlNode::new("instance")
                    .attr("id", "__last-saved")
                    .attr("src", "jr://instance/last-saved"),
            );
        }
        // pyxform emits an instance for every list in the choices sheet,
        // referenced or not — unless its name clashes with an external id
        // or the list is consumed by a search() select (inline items).
        for list in &survey.choice_lists {
            if seen.contains_key(&list.name) || self.searched_lists.contains(&list.name) {
                continue;
            }
            let needs_itext = list.needs_itext();
            let mut root = XmlNode::new("root");
            for (i, choice) in list.choices.iter().enumerate() {
                let mut item = XmlNode::new("item");
                if needs_itext {
                    item.push(XmlNode::new("itextId").text(&format!("{}-{}", list.name, i)));
                    item.push(XmlNode::new("name").text(&choice.name));
                } else {
                    item.push(XmlNode::new("name").text(&choice.name));
                    let label = self
                        .tr(&choice.label, &self.default_language)
                        .unwrap_or_default();
                    item.push(XmlNode::new("label").text(label));
                }
                for (column, value) in &choice.extras {
                    item.push(XmlNode::new(column).text(value));
                }
                root.push(item);
            }
            out.push(XmlNode::new("instance").attr("id", &list.name).child(root));
        }
        Ok(out)
    }

    // -- binds ---------------------------------------------------------------

    fn build_binds(&self, survey: &Survey) -> Result<Vec<XmlNode>> {
        let mut binds = Vec::new();
        self.collect_binds(&survey.items, &format!("/{}", survey.name), &mut binds)?;

        if let Some(entity) = &survey.entity {
            let entity_path = format!("/{}/meta/entity", survey.name);
            let ro_bind = |nodeset: &str| {
                XmlNode::new("bind")
                    .attr("nodeset", nodeset)
                    .attr("readonly", "true()")
                    .attr("type", "string")
            };
            if let Some(create_if) = &entity.create_if {
                binds.push(
                    ro_bind(&format!("{entity_path}/@create"))
                        .attr("calculate", &self.expand(create_if)?),
                );
            }
            if let Some(update_if) = &entity.update_if {
                binds.push(
                    ro_bind(&format!("{entity_path}/@update"))
                        .attr("calculate", &self.expand(update_if)?),
                );
            }
            if let Some(entity_id) = &entity.entity_id {
                // offline-entity version tracking against the dataset instance
                let id_expr = self.expand(entity_id)?;
                for (attr, column) in [
                    ("baseVersion", "__version"),
                    ("trunkVersion", "__trunkVersion"),
                    ("branchId", "__branchId"),
                ] {
                    binds.push(ro_bind(&format!("{entity_path}/@{attr}")).attr(
                        "calculate",
                        &format!(
                            "instance('{}')/root/item[name={id_expr}]/{column}",
                            entity.dataset
                        ),
                    ));
                }
                binds.push(ro_bind(&format!("{entity_path}/@id")).attr("calculate", &id_expr));
            } else {
                binds.push(ro_bind(&format!("{entity_path}/@id")));
                binds.push(
                    XmlNode::new("setvalue")
                        .attr("ref", &format!("{entity_path}/@id"))
                        .attr("event", "odk-instance-first-load")
                        .attr("value", "uuid()"),
                );
            }
            if let Some(label) = &entity.label {
                binds.push(
                    ro_bind(&format!("{entity_path}/label"))
                        .attr("calculate", &self.expand(label)?),
                );
            }
        }

        // audit binds sit with the meta block, right before instanceID
        for (_, q) in survey.walk() {
            if q.qtype == "audit" {
                let mut bind = XmlNode::new("bind")
                    .attr("nodeset", &format!("/{}/meta/{}", survey.name, q.name))
                    .attr("type", "binary");
                for (key, value) in &q.parameters {
                    bind = bind.attr(&format!("odk:{key}"), value);
                }
                binds.push(bind);
            }
        }

        if !survey.settings.omit_instance_id {
            let instance_id_bind = XmlNode::new("bind")
                .attr("jr:preload", "uid")
                .attr("nodeset", &format!("/{}/meta/instanceID", survey.name))
                .attr("readonly", "true()")
                .attr("type", "string");
            binds.push(instance_id_bind);
        }

        if let Some(expr) = &survey.settings.instance_name {
            binds.push(
                XmlNode::new("bind")
                    .attr("calculate", &self.expand(expr)?)
                    .attr("nodeset", &format!("/{}/meta/instanceName", survey.name))
                    .attr("type", "string"),
            );
        }
        Ok(binds)
    }

    fn collect_binds(&self, items: &[Item], prefix: &str, out: &mut Vec<XmlNode>) -> Result<()> {
        for item in items {
            match item {
                Item::Question(q) => {
                    let xpath = format!("{prefix}/{}", q.name);
                    if let Some(bind) = self.question_bind(q, &xpath)? {
                        out.push(bind);
                    }
                    // model-level actions ride along right after the bind
                    if q.default_is_dynamic() {
                        let default = q.default.as_deref().unwrap_or_default();
                        out.push(
                            XmlNode::new("setvalue")
                                .attr("ref", &xpath)
                                .attr("event", "odk-instance-first-load")
                                .attr("value", &self.expand(default)?),
                        );
                    }
                    match q.qtype.as_str() {
                        "start-geopoint" => out.push(
                            XmlNode::new("odk:setgeopoint")
                                .attr("ref", &xpath)
                                .attr("event", "odk-instance-first-load"),
                        ),
                        "background-audio" => {
                            let mut node = XmlNode::new("odk:recordaudio")
                                .attr("ref", &xpath)
                                .attr("event", "odk-instance-load");
                            if let Some(quality) = q.parameters.get("quality") {
                                node = node.attr("odk:quality", quality);
                            }
                            out.push(node);
                        }
                        _ => {}
                    }
                }
                Item::Section(s) => {
                    let xpath = Survey::child_prefix(s, prefix);
                    // a flat group has no node to bind; its relevant is
                    // dropped, matching pyxform
                    if !s.flat {
                        if let Some(relevant) = &s.relevant {
                            out.push(
                                XmlNode::new("bind")
                                    .attr("nodeset", &xpath)
                                    .attr("relevant", &self.expand(relevant)?),
                            );
                        }
                    }
                    self.collect_binds(&s.children, &xpath, out)?;
                }
            }
        }
        Ok(())
    }

    /// Returns `None` when the bind would carry nothing but its nodeset
    /// (e.g. a plain `trigger`), matching pyxform which omits it entirely.
    fn question_bind(&self, q: &Question, xpath: &str) -> Result<Option<XmlNode>> {
        if matches!(q.qtype.as_str(), "xml-external" | "csv-external" | "audit") {
            return Ok(None);
        }
        let def = self.type_def(q);
        let mut bind = XmlNode::new("bind").attr("nodeset", xpath);

        // media-capture parameters become namespaced bind attributes
        match def.mediatype {
            Some("image/*") => {
                if let Some(v) = q.parameters.get("max-pixels") {
                    bind = bind.attr("orx:max-pixels", v);
                }
            }
            Some("audio/*") => {
                if let Some(v) = q.parameters.get("quality") {
                    bind = bind.attr("odk:quality", v);
                }
            }
            _ => {}
        }

        let bind_type = match &q.select {
            Some((SelectKind::Rank, _)) => Some("odk:rank"),
            Some(_) => Some("string"),
            None => def.bind_type,
        };
        if let Some(bind_type) = bind_type {
            bind = bind.attr("type", bind_type);
        }

        if def.readonly {
            bind = bind.attr("readonly", "true()");
        } else if let Some(ro) = &q.readonly {
            bind = bind.attr("readonly", ro);
        }
        if let Some((preload, params)) = def.preload {
            bind = bind
                .attr("jr:preload", preload)
                .attr("jr:preloadParams", params);
        }
        if let Some(required) = &q.required {
            bind = bind.attr("required", required);
        }
        if let Some(relevant) = &q.relevant {
            bind = bind.attr("relevant", &self.expand(relevant)?);
        }
        match (&q.constraint, def.constraint) {
            (Some(c), _) => bind = bind.attr("constraint", &self.expand(c)?),
            (None, Some(c)) => bind = bind.attr("constraint", c),
            (None, None) => {}
        }
        // with a trigger column, the calculation fires as a setvalue action
        // in the triggering question's body instead of a bind calculate
        if q.trigger.is_none() {
            if let Some(calc) = &q.calculation {
                bind = bind.attr("calculate", &self.expand(calc)?);
            }
        }
        for (key, map, id) in [
            (
                "jr:constraintMsg",
                &q.constraint_message,
                format!("{xpath}:jr:constraintMsg"),
            ),
            (
                "jr:requiredMsg",
                &q.required_message,
                format!("{xpath}:jr:requiredMsg"),
            ),
        ] {
            if self.itext_ids.contains(&id) {
                bind = bind.attr(key, &format!("jr:itext('{id}')"));
            } else if let Some(msg) = self.tr(map, &self.default_language) {
                bind = bind.attr(key, msg);
            }
        }
        for (key, value) in &q.bind_extra {
            if key != "repeat_count" {
                bind = bind.attr(key, &self.expand(value)?);
            }
        }
        if bind.attrs.len() == 1 {
            return Ok(None);
        }
        Ok(Some(bind))
    }

    // -- body ----------------------------------------------------------------

    fn build_control(&self, survey: &Survey, item: &Item, prefix: &str) -> Result<Option<XmlNode>> {
        match item {
            Item::Question(q) => self.question_control(survey, q, prefix),
            Item::Section(s) => {
                let xpath = Survey::child_prefix(s, prefix);
                let mut children = Vec::new();
                // guard on the section's own label: flat groups collapse to a
                // shared xpath, so the itext id may exist thanks to a sibling
                if !s.label.is_empty() {
                    if let Some(label) = self.label_node(&s.label, &BTreeMap::new(), &xpath)? {
                        children.push(label);
                    }
                }
                match s.kind {
                    SectionKind::Loop => unreachable!("loops are expanded by the parser"),
                    SectionKind::Group => {
                        let mut group = XmlNode::new("group");
                        // flat groups keep their body element but have no
                        // instance node to point at
                        if !s.flat {
                            group = group.attr("ref", &xpath);
                        }
                        if let Some(appearance) = &s.appearance {
                            group = group.attr("appearance", appearance);
                        }
                        for c in children {
                            group.push(c);
                        }
                        for child in &s.children {
                            if let Some(n) = self.build_control(survey, child, &xpath)? {
                                group.push(n);
                            }
                        }
                        Ok(Some(group))
                    }
                    SectionKind::Repeat => {
                        // pyxform always gives the wrapping group a label,
                        // empty when the repeat row has none.
                        if children.is_empty() {
                            children.push(XmlNode::new("label").text(""));
                        }
                        let mut repeat = XmlNode::new("repeat").attr("nodeset", &xpath);
                        if let Some(count) = &s.count {
                            repeat = repeat.attr("jr:count", &self.expand_padded(count)?);
                        }
                        if let Some(appearance) = &s.appearance {
                            repeat = repeat.attr("appearance", appearance);
                        }
                        for child in &s.children {
                            if let Some(n) = self.build_control(survey, child, &xpath)? {
                                repeat.push(n);
                            }
                        }
                        // dynamic defaults of this repeat's own questions
                        // also fire on each new repeat instance
                        self.push_new_repeat_setvalues(&s.children, &xpath, &mut repeat)?;
                        let mut group = XmlNode::new("group").attr("ref", &xpath);
                        for c in children {
                            group.push(c);
                        }
                        group.push(repeat);
                        Ok(Some(group))
                    }
                }
            }
        }
    }

    /// Expand `${x}` but, matching pyxform's repeat-count output, keep the
    /// surrounding spaces even when the expression is a lone reference.
    fn expand_padded(&self, expr: &str) -> Result<String> {
        self.expand(expr)
    }

    /// `<setvalue event="odk-new-repeat">` entries appended at the end of a
    /// repeat body: one per dynamic default among the repeat's descendants,
    /// stopping at nested repeats (which handle their own).
    fn push_new_repeat_setvalues(
        &self,
        items: &[Item],
        prefix: &str,
        repeat: &mut XmlNode,
    ) -> Result<()> {
        for item in items {
            match item {
                Item::Question(q) if q.default_is_dynamic() => {
                    let default = q.default.as_deref().unwrap_or_default();
                    repeat.push(
                        XmlNode::new("setvalue")
                            .attr("ref", &format!("{prefix}/{}", q.name))
                            .attr("event", "odk-new-repeat")
                            .attr("value", &self.expand(default)?),
                    );
                }
                Item::Question(_) => {}
                Item::Section(s) if s.kind == SectionKind::Group => {
                    self.push_new_repeat_setvalues(
                        &s.children,
                        &format!("{prefix}/{}", s.name),
                        repeat,
                    )?;
                }
                Item::Section(_) => {} // nested repeats handle their own
            }
        }
        Ok(())
    }

    fn question_control(
        &self,
        survey: &Survey,
        q: &Question,
        prefix: &str,
    ) -> Result<Option<XmlNode>> {
        let def = self.type_def(q);
        let Some(tag) = def.control_tag else {
            return Ok(None);
        };
        if tag == "action" {
            // odk actions (start-geopoint etc.) live in the model in the full
            // spec; rxform does not emit them yet.
            return Ok(None);
        }
        let xpath = format!("{prefix}/{}", q.name);
        let mut control = XmlNode::new(tag).attr("ref", &xpath);
        if let Some(mediatype) = def.mediatype {
            control = control.attr("mediatype", mediatype);
        }
        if let Some(appearance) = &q.appearance {
            control = control.attr("appearance", &self.expand(appearance)?);
        }
        if q.qtype == "range" {
            for (key, default) in [("start", "1"), ("end", "10"), ("step", "1")] {
                let value = q.parameters.get(key).map(String::as_str).unwrap_or(default);
                control = control.attr(key, value);
            }
        }
        if let Some(rows) = q.parameters.get("rows") {
            control = control.attr("rows", rows);
        }
        if let Some(autoplay) = q.parameters.get("autoplay") {
            control = control.attr("autoplay", autoplay);
        }
        if matches!(q.qtype.as_str(), "geopoint" | "gps" | "location") {
            if let Some(v) = q.parameters.get("capture-accuracy") {
                control = control.attr("accuracyThreshold", v);
            }
            if let Some(v) = q.parameters.get("warning-accuracy") {
                control = control.attr("unacceptableAccuracyThreshold", v);
            }
        }
        for (key, value) in &q.body_attrs {
            control = control.attr(key, &self.expand(value)?);
        }
        // select_one_external queries the itemsets.csv sideloaded by clients
        if q.qtype == "select one external" {
            if let (Some((_, list_name)), Some(filter)) = (&q.select, &q.choice_filter) {
                control = control.attr(
                    "query",
                    &format!(
                        "instance('{list_name}')/root/item[{}]",
                        self.expand(filter)?
                    ),
                );
            }
        }

        match self.label_node(&q.label, &q.media, &xpath)? {
            Some(label) => control.push(label),
            // select controls always carry a label element, even when empty
            None if q.select.is_some() => control.push(XmlNode::new("label").text("")),
            None => {}
        }
        if let Some(hint) = self.hint_node(q, &xpath)? {
            control.push(hint);
        }

        if let Some(items) = self.search_items(survey, q)? {
            for item in items {
                control.push(item);
            }
        } else if let Some(itemset) = self.itemset_node(survey, q)? {
            control.push(itemset);
        }

        // trigger-column actions fire from the source question's control
        if let Some(actions) = self.triggers.get(&q.name) {
            for action in actions {
                let mut node = if action.geopoint {
                    XmlNode::new("odk:setgeopoint")
                } else {
                    XmlNode::new("setvalue")
                };
                node = node
                    .attr("ref", &action.target)
                    .attr("event", "xforms-value-changed");
                if let Some(value) = &action.value {
                    node = node.attr("value", value);
                }
                control.push(node);
            }
        }
        Ok(Some(control))
    }

    /// Inline `<item>` elements for selects using the `search()` appearance:
    /// the choice rows act as column templates for the searched CSV.
    fn search_items(&self, survey: &Survey, q: &Question) -> Result<Option<Vec<XmlNode>>> {
        let Some((_, list_name)) = &q.select else {
            return Ok(None);
        };
        if !self.searched_lists.contains(list_name) {
            return Ok(None);
        }
        let Some(list) = survey.choice_list(list_name) else {
            return Ok(None);
        };
        let needs_itext = list.needs_itext();
        let mut items = Vec::new();
        for (i, choice) in list.choices.iter().enumerate() {
            let label = if needs_itext {
                XmlNode::new("label").attr("ref", &format!("jr:itext('{}-{}')", list.name, i))
            } else {
                match self.tr(&choice.label, &self.default_language) {
                    Some(text) => {
                        let (fragment, has_output) = self.expand_label(text)?;
                        if has_output {
                            XmlNode::new("label").raw_text(&fragment)
                        } else {
                            XmlNode::new("label").text(text)
                        }
                    }
                    None => XmlNode::new("label"),
                }
            };
            items.push(
                XmlNode::new("item")
                    .child(label)
                    .child(XmlNode::new("value").text(&choice.name)),
            );
        }
        Ok(Some(items))
    }

    /// The `<itemset>` for a select: from an internal choice-list instance,
    /// or from an external file for `select_*_from_file`.
    fn itemset_node(&self, survey: &Survey, q: &Question) -> Result<Option<XmlNode>> {
        if q.qtype == "select one external" {
            return Ok(None);
        }
        let Some((_, list_name)) = &q.select else {
            return Ok(None);
        };
        let (mut nodeset, value_ref, label_ref);
        if let Some((base, ext)) = q.select_from_file() {
            let (v, l) = if ext == "geojson" {
                ("id", "title")
            } else {
                ("name", "label")
            };
            value_ref = q
                .parameters
                .get("value")
                .cloned()
                .unwrap_or_else(|| v.into());
            label_ref = q
                .parameters
                .get("label")
                .cloned()
                .unwrap_or_else(|| l.into());
            nodeset = format!("instance('{base}')/root/item");
        } else if let Some(list) = survey.choice_list(list_name) {
            value_ref = "name".to_string();
            label_ref = if list.needs_itext() {
                "jr:itext(itextId)".to_string()
            } else {
                "label".to_string()
            };
            nodeset = format!("instance('{}')/root/item", list.name);
        } else {
            return Ok(None);
        };
        if let Some(filter) = &q.choice_filter {
            nodeset = format!("{nodeset}[{}]", self.expand(filter)?);
        }
        if q.parameters.get("randomize").map(String::as_str) == Some("true") {
            nodeset = match q.parameters.get("seed") {
                Some(seed) => format!("randomize({nodeset}, {})", self.expand(seed)?.trim()),
                None => format!("randomize({nodeset})"),
            };
        }
        Ok(Some(
            XmlNode::new("itemset")
                .attr("nodeset", &nodeset)
                .child(XmlNode::new("value").attr("ref", &value_ref))
                .child(XmlNode::new("label").attr("ref", &label_ref)),
        ))
    }

    fn label_node(
        &self,
        label: &Translated,
        media: &BTreeMap<String, Translated>,
        xpath: &str,
    ) -> Result<Option<XmlNode>> {
        let id = format!("{xpath}:label");
        if self.itext_ids.contains(&id) {
            return Ok(Some(
                XmlNode::new("label").attr("ref", &format!("jr:itext('{id}')")),
            ));
        }
        let _ = media;
        match self.tr(label, &self.default_language) {
            Some(text) => {
                let (fragment, has_output) = self.expand_label(text)?;
                Ok(Some(if has_output {
                    XmlNode::new("label").raw_text(&fragment)
                } else {
                    XmlNode::new("label").text(text)
                }))
            }
            None => Ok(None),
        }
    }

    fn hint_node(&self, q: &Question, xpath: &str) -> Result<Option<XmlNode>> {
        let id = format!("{xpath}:hint");
        if self.itext_ids.contains(&id) {
            return Ok(Some(
                XmlNode::new("hint").attr("ref", &format!("jr:itext('{id}')")),
            ));
        }
        match self.tr(&q.hint, &self.default_language) {
            Some(text) => Ok(Some(XmlNode::new("hint").text(text))),
            None => match self.type_def(q).hint {
                Some(hint) => Ok(Some(XmlNode::new("hint").text(hint))),
                None => Ok(None),
            },
        }
    }

    fn type_def(&self, q: &Question) -> TypeDef {
        types::lookup(&q.qtype).unwrap_or_default()
    }
}

/// The `(instance id, jr:// URI)` pairs a single question contributes:
/// `pulldata('x', ...)` calls in its expressions, a `select_*_from_file`
/// source, or an `xml-external`/`csv-external` row.
fn external_instances_for(q: &Question) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let expressions = [
        q.calculation.as_ref(),
        q.constraint.as_ref(),
        q.relevant.as_ref(),
        q.required.as_ref(),
        q.choice_filter.as_ref(),
        q.default.as_ref(),
    ];
    for expr in expressions.into_iter().flatten() {
        for id in pulldata_file_ids(expr) {
            let src = format!("jr://file-csv/{id}.csv");
            out.push((id, src));
        }
    }
    if let Some((base, ext)) = q.select_from_file() {
        let prefix = if ext == "csv" { "file-csv" } else { "file" };
        let (_, filename) = q.select.as_ref().expect("from_file implies select");
        out.push((base.to_string(), format!("jr://{prefix}/{filename}")));
    }
    match q.qtype.as_str() {
        "xml-external" => out.push((q.name.clone(), format!("jr://file/{}.xml", q.name))),
        "csv-external" => out.push((q.name.clone(), format!("jr://file-csv/{}.csv", q.name))),
        _ => {}
    }
    out
}

/// First arguments of every `pulldata(...)` call in an expression.
fn pulldata_file_ids(expr: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = expr;
    while let Some(pos) = rest.find("pulldata(") {
        let after = &rest[pos + "pulldata(".len()..];
        let arg_end = after.find([',', ')']).unwrap_or(after.len());
        let id = after[..arg_end].trim().trim_matches(['\'', '"']).trim();
        if !id.is_empty() {
            out.push(id.to_string());
        }
        rest = after;
    }
    out
}

fn build_submission(settings: &Settings) -> Option<XmlNode> {
    if settings.submission_url.is_none()
        && settings.public_key.is_none()
        && settings.auto_send.is_none()
        && settings.auto_delete.is_none()
    {
        return None;
    }
    let mut node = XmlNode::new("submission");
    if let Some(url) = &settings.submission_url {
        node = node.attr("action", url).attr("method", "post");
    }
    if let Some(key) = &settings.public_key {
        node = node.attr("base64RsaPublicKey", key);
    }
    if let Some(v) = &settings.auto_send {
        node = node.attr("orx:auto-send", v);
    }
    if let Some(v) = &settings.auto_delete {
        node = node.attr("orx:auto-delete", v);
    }
    Some(node)
}

fn has_translation(map: &Translated) -> bool {
    map.keys().any(|lang| lang != DEFAULT_LANG)
}

/// Media kinds in pyxform's emission order (not alphabetical).
fn ordered_media(media: &BTreeMap<String, Translated>) -> Vec<(&str, &Translated)> {
    ["image", "big-image", "audio", "video"]
        .iter()
        .filter_map(|kind| media.get(*kind).map(|tr| (*kind, tr)))
        .collect()
}

fn media_uri(kind: &str, value: &str) -> String {
    if value.contains("://") {
        return value.to_string();
    }
    match kind {
        "image" | "big-image" => format!("jr://images/{value}"),
        other => format!("jr://{other}/{value}"),
    }
}

/// Every labeled element (questions and sections) in document order, with
/// its absolute xpath.
enum ElementRef<'a> {
    Question(&'a Question),
    Section(&'a Section),
}

impl<'a> ElementRef<'a> {
    #[allow(clippy::type_complexity)]
    fn translatables(
        &self,
    ) -> (
        &'a Translated,
        &'a Translated,
        &'a Translated,
        &'a BTreeMap<String, Translated>,
        &'a Translated,
        &'a Translated,
    ) {
        const EMPTY_TR: &Translated = &BTreeMap::new();
        static EMPTY_MEDIA: BTreeMap<String, Translated> = BTreeMap::new();
        match self {
            ElementRef::Question(q) => (
                &q.label,
                &q.hint,
                &q.guidance_hint,
                &q.media,
                &q.constraint_message,
                &q.required_message,
            ),
            ElementRef::Section(s) => (
                &s.label,
                EMPTY_TR,
                EMPTY_TR,
                &EMPTY_MEDIA,
                EMPTY_TR,
                EMPTY_TR,
            ),
        }
    }
}

fn document_order<'a>(items: &'a [Item], prefix: &str) -> Vec<(String, ElementRef<'a>)> {
    let mut out = Vec::new();
    for item in items {
        match item {
            Item::Question(q) => {
                out.push((format!("{prefix}/{}", q.name), ElementRef::Question(q)));
            }
            Item::Section(s) => {
                let xpath = Survey::child_prefix(s, prefix);
                out.push((xpath.clone(), ElementRef::Section(s)));
                out.extend(document_order(&s.children, &xpath));
            }
        }
    }
    out
}
