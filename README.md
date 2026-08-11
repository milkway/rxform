# rxform

[![crates.io](https://img.shields.io/crates/v/rxform.svg)](https://crates.io/crates/rxform)
[![docs.rs](https://docs.rs/rxform/badge.svg)](https://docs.rs/rxform)
[![license: BSD-2-Clause](https://img.shields.io/badge/license-BSD--2--Clause-blue.svg)](LICENSE)
[![DOI](https://zenodo.org/badge/DOI/10.5281/zenodo.21894015.svg)](https://doi.org/10.5281/zenodo.21894015)

A Rust implementation of [pyxform](https://github.com/xlsform/pyxform): converts
[XLSForm](https://xlsform.org) spreadsheets into ODK
[XForm](https://getodk.github.io/xforms-spec/) XML, as used by ODK Collect,
Enketo, KoboToolbox and friends. Ships as a CLI and as a library crate.

Output is validated against **pyxform 4.5.0**: for every form in
`tests/fixtures/` — the pyxform example forms plus one form per advanced
feature — rxform's output is identical to pyxform's after XML canonicalization
(attribute order aside). The one exception is the ordering of `<text>` entries
inside `<translation>` blocks in heavily multilingual forms, which has no
semantic effect.

## Usage

```sh
# writes survey.xml next to the input
rxform survey.xlsx

# explicit output path, or stdout
rxform survey.xlsx -o out/form.xml
rxform survey.xlsx --stdout
```

As a library:

```rust
let xml = rxform::convert_file(std::path::Path::new("survey.xlsx"))?;
```

Input formats: `.xlsx`, `.xls`, `.ods`
(via [calamine](https://crates.io/crates/calamine)).

## How it works

![rxform pipeline](https://raw.githubusercontent.com/milkway/rxform/main/docs/pipeline.svg?v=3)

The workbook is read and normalized (`xls.rs`), parsed into a tree of
questions, groups and choice lists (`parser.rs` + `model.rs`), validated with
precise locations (`parser/validate.rs`), and finally serialized into the
XForm document (`xform.rs` + `xmlwriter.rs`) using pyxform's output
conventions, so existing ODK tooling sees familiar XML.

The second diagram shows where each piece of the spreadsheet ends up in the
generated document:

![XLSForm to XForm mapping](https://raw.githubusercontent.com/milkway/rxform/main/docs/mapping.svg?v=2)

## Error reporting

Broken forms fail with the sheet, row and column of the problem and a probable
cause — never a stack trace or a silently wrong form:

```
$ rxform broken.xlsx
error: [sheet 'survey', row 3, column 'type'] unknown question type 'integr'
  probable cause: did you mean 'integer'?
```

Checks include, among others:

- unknown question types and misspelled select prefixes ("did you mean
  'select_one'?");
- `${references}` that don't match any field, with the closest name suggested;
- selects pointing at nonexistent choice lists (closest list suggested, or the
  available lists shown);
- duplicate or invalid (non-XML) names, with both offending rows;
- unclosed `begin group`/`begin repeat` (points at the `begin` row) and
  mismatched or orphan `end` rows;
- ambiguous `${references}` (the same name in two different sections) at the
  place of use — duplicated names between sections are otherwise legal, as
  loops rely on them;
- entities-sheet mistakes (`update_if` without `entity_id`, creation without
  a label, unknown columns) and `save_to` inside repeats;
- loops over nonexistent choice lists;
- `trigger` values that aren't a `${question}` reference, point at nothing, or
  point at a non-user-visible question (calculations can't fire triggers);
- `background-geopoint` without a trigger;
- choice names with spaces in `select_multiple` lists;
- duplicate choice names in a list (unless `allow_choice_duplicates` is set);
- visible questions with no label, hint or media;
- `or_other` combined with `choice_filter` or from-file selects;
- two external data sources claiming the same instance id with different
  files.

## What is supported

- `survey`, `choices` and `settings` sheets (case-insensitive names).
- Question types: text/string, integer, decimal, range, date, time, dateTime,
  note, trigger/acknowledge, geopoint/geotrace/geoshape, photo/image, audio,
  video, file, osm, barcode, calculate, hidden, audit, rank, and the metadata
  preloads (start, end, today, deviceid, phonenumber, username, email, etc.).
- `select_one` / `select_multiple` / `rank` with choice lists, rendered as
  secondary instances + `<itemset>`, including `choice_filter` predicates,
  extra choice columns, and `randomize`/`seed` parameters.
- `select_one_from_file` / `select_multiple_from_file` (csv, xml, geojson;
  `value=`/`label=` parameter overrides), `xml-external`/`csv-external` rows,
  and `pulldata()` calls — each becomes an
  `<instance id src="jr://file…"/>` with pyxform's dedup rules.
- `select_one_external`: the `choice_filter` becomes the control's `query`
  attribute and the `external_choices` sheet is exported as `itemsets.csv`
  next to the output file.
- `or_other`: appends the Other choice (translated when the list is) and
  generates the "Specify other." follow-up question with its relevance.
- Groups and repeats (`begin/end group|repeat`), nested arbitrarily;
  `repeat_count` (a generated `<name>_count` calculate node is created for
  non-trivial expressions, exactly like pyxform).
- The `table-list` appearance transformation (generated label note + header
  select + `list-nolabel`).
- Bind columns: `relevant`, `constraint`, `constraint_message`, `required`,
  `required_message`, `read_only`, `calculation`, plus passthrough `bind::*`
  columns and the common aliases (`bind:jr:constraintMsg`,
  `control:appearance`, `repeat_count`, `rows`, `autoplay`,
  `noAppErrorString`, ...).
- Generic column prefixes: `body::attr` (any control attribute),
  `instance::attr` (attributes on the question's instance node), and in
  settings `attribute::x` (primary-instance root attributes) and `namespaces`
  (extra `xmlns:` declarations).
- Capture parameters: image `max-pixels` (`orx:max-pixels` bind), audio
  `quality` (`odk:quality` on the bind, or on `<odk:recordaudio>` for
  background-audio), geopoint `capture-accuracy`/`warning-accuracy`
  (`accuracyThreshold`/`unacceptableAccuracyThreshold` body attributes).
- `audit`, placed at `meta/audit` with its `location-priority`,
  `location-min-interval` and `location-max-age` parameters as `odk:*` bind
  attributes.
- The `entities` sheet: create, update and conditional create+update
  declarations (`list_name`, `entity_id`, `create_if`, `update_if`, `label`),
  the `save_to` column (`entities:saveto` binds), the `meta/entity` block
  with offline `baseVersion`/`trunkVersion`/`branchId` tracking, and the
  `entities:entities-version` model attribute.
- The `search()` appearance: choices become inline `<item>` column templates
  and the list is excluded from the secondary instances.
- `${last-saved#question}` references, expanding to the
  `jr://instance/last-saved` secondary instance.
- Compact/SMS record representation: settings `prefix`/`delimiter`
  (`odk:prefix`/`odk:delimiter`) and the `compact_tag` column (`odk:tag`).
- `flat` forms (settings `flat=yes`): groups keep their body element but
  vanish from the instance and xpaths, with group relevance "and"-ed down
  onto their questions; plus `omit_instanceID`.
- `begin loop over <list>`: expands into one sub-group per choice with
  `%(name)s` / `%(label)s` substituted per language.
- `clean_text_values` (default on, like pyxform): collapses runs of spaces
  and trims every cell; `clean_text_values=no` preserves spacing.
- Defaults: static values go into the primary instance; dynamic expressions
  become `<setvalue event="odk-instance-first-load">` actions (plus
  `odk-new-repeat` inside repeats), like pyxform.
- The `trigger` column: `<setvalue event="xforms-value-changed">` fired from
  the source question, carrying the target's calculation (or clearing it).
- odk actions: `start-geopoint` (`<odk:setgeopoint>` on first load),
  `background-audio` (`<odk:recordaudio>`), `background-geopoint`
  (geopoint captured when its trigger changes).
- `${name}` reference expansion in expressions and labels (labels get inline
  `<output value="..."/>` elements).
- Multiple languages via `column::Language` (and the legacy `column:lang`),
  `media::image/audio/video/big-image`, `guidance_hint`, all routed through
  `<itext>` with pyxform's rules (untranslated content stays inline; missing
  translations become `-`).
- Notes without a `name` get pyxform's auto-generated names.
- Settings: `form_title`, `form_id`, `name` (instance root, default `data`),
  `version`, `default_language`, `instance_name`, `style`, `submission_url`,
  `public_key`, `auto_send`, `auto_delete`.
- Smart-quote straightening in all cell values.
- `meta/instanceID` (and `instanceName` when `instance_name` is set).

## Not implemented

- OSM tag lists (the `osm` upload type itself is supported).
- The legacy J2ME `sms_*` columns (the modern compact/`odk:tag`
  representation is supported).

## Installation

```sh
cargo install rxform        # CLI
cargo add rxform            # library
```

## Development

```sh
cargo test
```

`tests/fixtures/` contains example forms copied from the pyxform test suite
(BSD 2-Clause, © the pyxform contributors) plus small feature-specific forms.
`tests/expected/` holds rxform's output for them, validated against pyxform
4.5.0 by canonical XML comparison; the integration tests pin rxform to those
snapshots, and `tests/errors_test.rs` locks in the diagnostics (location +
probable cause) for common authoring mistakes.

## License and citation

rxform is open source under the [BSD 2-Clause License](LICENSE), the same
license as pyxform, whose behavior it reimplements and whose test forms it
reuses. Contributions — issues, example forms that convert differently from
pyxform, and pull requests — are welcome at
[github.com/milkway/rxform](https://github.com/milkway/rxform).

To cite rxform in academic work, use the metadata in
[CITATION.cff](CITATION.cff) or the Zenodo archive:
[doi:10.5281/zenodo.21894015](https://doi.org/10.5281/zenodo.21894015)
(concept DOI — always resolves to the latest release).
