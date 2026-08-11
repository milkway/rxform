//! Graceful-failure tests: every authoring mistake must fail with the sheet,
//! row and column where it happened, plus a probable cause where we have one.

use rxform::xls::{sheet_from_rows, Sheet, Workbook};

fn sheet(rows: &[&[&str]]) -> Sheet {
    sheet_from_rows(
        rows.iter()
            .map(|r| r.iter().map(|c| c.to_string()).collect()),
    )
}

fn convert(survey: &[&[&str]], choices: &[&[&str]]) -> Result<String, rxform::Error> {
    let wb = Workbook {
        survey: sheet(survey),
        choices: sheet(choices),
        ..Default::default()
    };
    rxform::convert_workbook(&wb, "t")
}

fn err_of(survey: &[&[&str]], choices: &[&[&str]]) -> String {
    convert(survey, choices)
        .expect_err("expected the form to be rejected")
        .to_string()
}

#[test]
fn unknown_type_points_at_row_and_suggests() {
    let msg = err_of(&[&["type", "name"], &["text", "a"], &["integr", "b"]], &[]);
    assert!(msg.contains("sheet 'survey'"), "{msg}");
    assert!(msg.contains("row 3"), "{msg}");
    assert!(msg.contains("column 'type'"), "{msg}");
    assert!(msg.contains("did you mean 'integer'?"), "{msg}");
}

#[test]
fn misspelled_select_prefix_is_suggested() {
    let msg = err_of(&[&["type", "name"], &["selectone yn", "q"]], &[]);
    assert!(msg.contains("did you mean 'select_one'?"), "{msg}");
}

#[test]
fn missing_choice_list_suggests_closest_name() {
    let msg = err_of(
        &[&["type", "name"], &["select_one colors", "q"]],
        &[&["list_name", "name", "label"], &["colours", "red", "Red"]],
    );
    assert!(msg.contains("row 2"), "{msg}");
    assert!(msg.contains("'colors'"), "{msg}");
    assert!(msg.contains("did you mean 'colours'?"), "{msg}");
}

#[test]
fn unresolved_reference_points_at_column_and_suggests() {
    let msg = err_of(
        &[
            &["type", "name", "relevant"],
            &["integer", "age", ""],
            &["text", "child", "${aeg} < 18"],
        ],
        &[],
    );
    assert!(msg.contains("row 3"), "{msg}");
    assert!(msg.contains("column 'relevant'"), "{msg}");
    assert!(msg.contains("did you mean 'age'?"), "{msg}");
}

#[test]
fn unterminated_reference_is_reported() {
    let msg = err_of(
        &[
            &["type", "name", "calculation"],
            &["calculate", "c", "${age + 1"],
        ],
        &[],
    );
    assert!(msg.contains("unterminated"), "{msg}");
    assert!(msg.contains("row 2"), "{msg}");
}

#[test]
fn duplicate_names_report_both_rows() {
    let msg = err_of(
        &[&["type", "name"], &["text", "q1"], &["integer", "q1"]],
        &[],
    );
    assert!(msg.contains("rows 2 and 3"), "{msg}");
    assert!(msg.contains("unambiguous"), "{msg}");
}

#[test]
fn invalid_name_explains_the_rules() {
    let msg = err_of(&[&["type", "name"], &["text", "1st question"]], &[]);
    assert!(msg.contains("row 2"), "{msg}");
    assert!(msg.contains("column 'name'"), "{msg}");
    assert!(msg.contains("must start with a letter"), "{msg}");
}

#[test]
fn unclosed_group_points_at_the_begin_row() {
    let msg = err_of(
        &[&["type", "name"], &["begin group", "g1"], &["text", "q"]],
        &[],
    );
    assert!(msg.contains("row 2"), "{msg}");
    assert!(msg.contains("unclosed group named 'g1'"), "{msg}");
    assert!(msg.contains("add an 'end group'"), "{msg}");
}

#[test]
fn mismatched_end_names_the_open_section() {
    let msg = err_of(
        &[
            &["type", "name"],
            &["begin repeat", "r1"],
            &["text", "q"],
            &["end group", ""],
        ],
        &[],
    );
    assert!(
        msg.contains("'end group' does not match the open 'repeat' named 'r1'"),
        "{msg}"
    );
    assert!(msg.contains("started on row 2"), "{msg}");
}

#[test]
fn end_without_begin_has_a_hint() {
    let msg = err_of(&[&["type", "name"], &["end group", ""]], &[]);
    assert!(msg.contains("without a matching 'begin'"), "{msg}");
    assert!(msg.contains("probable cause"), "{msg}");
}

#[test]
fn trigger_must_be_a_reference() {
    let msg = err_of(
        &[
            &["type", "name", "label", "trigger"],
            &["integer", "age", "Age", ""],
            &["text", "t", "T", "age"],
        ],
        &[],
    );
    assert!(msg.contains("column 'trigger'"), "{msg}");
    assert!(
        msg.contains("${age}") || msg.contains("${question}"),
        "{msg}"
    );
}

#[test]
fn trigger_on_invisible_question_is_rejected() {
    let msg = err_of(
        &[
            &["type", "name", "label", "trigger", "calculation"],
            &["calculate", "c1", "", "", "1 + 1"],
            &["text", "t", "T", "${c1}", ""],
        ],
        &[],
    );
    assert!(msg.contains("not user-visible"), "{msg}");
}

#[test]
fn background_geopoint_requires_trigger() {
    let msg = err_of(&[&["type", "name"], &["background-geopoint", "loc"]], &[]);
    assert!(msg.contains("has no trigger"), "{msg}");
    assert!(msg.contains("column 'trigger'"), "{msg}");
}

#[test]
fn select_multiple_choice_names_cannot_contain_spaces() {
    let msg = err_of(
        &[&["type", "name"], &["select_multiple yn", "q"]],
        &[
            &["list_name", "name", "label"],
            &["yn", "not sure", "Not sure"],
        ],
    );
    assert!(msg.contains("sheet 'choices'"), "{msg}");
    assert!(msg.contains("row 2"), "{msg}");
    assert!(msg.contains("space"), "{msg}");
}

#[test]
fn or_other_with_choice_filter_is_rejected() {
    let msg = err_of(
        &[
            &["type", "name", "choice_filter"],
            &["select_one yn or_other", "q", "state=1"],
        ],
        &[&["list_name", "name", "label"], &["yn", "y", "Y"]],
    );
    assert!(msg.contains("or_other"), "{msg}");
    assert!(msg.contains("probable cause"), "{msg}");
}

#[test]
fn conflicting_external_instance_ids_are_rejected() {
    let msg = err_of(
        &[
            &["type", "name", "calculation"],
            &["xml-external", "fruits", ""],
            &["calculate", "c", "pulldata('fruits', 'a', 'b', 'c')"],
        ],
        &[],
    );
    assert!(msg.contains("instance id 'fruits'"), "{msg}");
    assert!(msg.contains("different files"), "{msg}");
}

#[test]
fn duplicate_choice_names_point_at_the_row() {
    let msg = err_of(
        &[&["type", "name", "label"], &["select_one yn", "q", "Q"]],
        &[
            &["list_name", "name", "label"],
            &["yn", "yes", "Yes"],
            &["yn", "yes", "Sim"],
        ],
    );
    assert!(msg.contains("sheet 'choices'"), "{msg}");
    assert!(msg.contains("rows 2 and 3"), "{msg}");
    assert!(msg.contains("allow_choice_duplicates"), "{msg}");
}

#[test]
fn visible_question_without_label_or_hint_is_rejected() {
    let msg = err_of(&[&["type", "name"], &["text", "quiet"]], &[]);
    assert!(msg.contains("row 2"), "{msg}");
    assert!(msg.contains("no label or hint"), "{msg}");
    assert!(msg.contains("'calculate' or 'hidden'"), "{msg}");
}

#[test]
fn label_only_in_hint_or_media_is_accepted() {
    let ok = convert(
        &[
            &["type", "name", "hint", "image"],
            &["text", "hinted", "just a hint", ""],
            &["text", "pictured", "", "pic.jpg"],
        ],
        &[],
    );
    assert!(ok.is_ok(), "{:?}", ok.err().map(|e| e.to_string()));
}

#[test]
fn ambiguous_reference_is_rejected_at_the_reference() {
    // same name in different groups is fine — until something points at it
    let msg = err_of(
        &[
            &["type", "name", "label"],
            &["begin group", "g1", "G1"],
            &["text", "city", "City"],
            &["end group", ""],
            &["begin group", "g2", "G2"],
            &["text", "city", "City"],
            &["end group", ""],
            &["note", "shown", "You said ${city}"],
        ],
        &[],
    );
    assert!(msg.contains("ambiguous"), "{msg}");
    assert!(msg.contains("row 8"), "{msg}");
}

#[test]
fn duplicate_names_in_different_groups_are_allowed() {
    let ok = convert(
        &[
            &["type", "name", "label"],
            &["begin group", "g1", "G1"],
            &["text", "city", "City"],
            &["end group", ""],
            &["begin group", "g2", "G2"],
            &["text", "city", "City"],
            &["end group", ""],
        ],
        &[],
    );
    assert!(ok.is_ok(), "{:?}", ok.err().map(|e| e.to_string()));
}

#[test]
fn loop_over_missing_list_is_rejected() {
    let msg = err_of(
        &[
            &["type", "name", "label"],
            &["begin loop over colors", "l1", ""],
            &["integer", "n", "How many %(label)s?"],
            &["end loop", ""],
        ],
        &[&["list_name", "name", "label"], &["colours", "red", "Red"]],
    );
    assert!(msg.contains("row 2"), "{msg}");
    assert!(msg.contains("did you mean 'colours'?"), "{msg}");
}

#[test]
fn entities_update_without_id_is_rejected() {
    let wb = Workbook {
        survey: sheet(&[&["type", "name", "label"], &["text", "q", "Q"]]),
        entities: sheet(&[&["list_name", "update_if"], &["trees", "true()"]]),
        ..Default::default()
    };
    let msg = rxform::convert_workbook(&wb, "t").unwrap_err().to_string();
    assert!(msg.contains("sheet 'entities'"), "{msg}");
    assert!(msg.contains("update_if requires an entity_id"), "{msg}");
}

#[test]
fn entities_create_without_label_is_rejected() {
    let wb = Workbook {
        survey: sheet(&[&["type", "name", "label"], &["text", "q", "Q"]]),
        entities: sheet(&[&["list_name"], &["trees"]]),
        ..Default::default()
    };
    let msg = rxform::convert_workbook(&wb, "t").unwrap_err().to_string();
    assert!(msg.contains("must give them a label"), "{msg}");
}

#[test]
fn save_to_inside_repeat_is_rejected() {
    let wb = Workbook {
        survey: sheet(&[
            &["type", "name", "label", "save_to"],
            &["begin repeat", "r", "R", ""],
            &["text", "sp", "Species", "species"],
            &["end repeat", "", "", ""],
        ]),
        entities: sheet(&[&["list_name", "label"], &["trees", "concat('a')"]]),
        ..Default::default()
    };
    let msg = rxform::convert_workbook(&wb, "t").unwrap_err().to_string();
    assert!(msg.contains("row 3"), "{msg}");
    assert!(msg.contains("inside a repeat"), "{msg}");
}

#[test]
fn missing_survey_sheet_is_explained() {
    let err = rxform::convert_file(std::path::Path::new("does-not-exist.xlsx"))
        .expect_err("expected an error");
    assert!(err.to_string().contains("cannot open workbook"), "{err}");
}
