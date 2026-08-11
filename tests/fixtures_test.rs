//! End-to-end tests: convert the pyxform example forms and compare with the
//! expected XForm output. The expected files were validated against pyxform
//! 4.5.0 (byte-identical after XML canonicalization; xlsform_spec_test also
//! matches modulo the ordering of <text> entries inside <translation>).

use std::path::Path;

fn check(name: &str) {
    let input = Path::new("tests/fixtures").join(format!("{name}.xlsx"));
    let expected_path = Path::new("tests/expected").join(format!("{name}.xml"));
    let actual =
        rxform::convert_file(&input).unwrap_or_else(|e| panic!("conversion of {name} failed: {e}"));
    let expected = std::fs::read_to_string(&expected_path)
        .unwrap_or_else(|e| panic!("missing expected output for {name}: {e}"));
    pretty_assertions::assert_eq!(actual, expected, "output mismatch for {name}");
}

#[test]
fn yes_or_no_question() {
    check("yes_or_no_question");
}

#[test]
fn text_and_integer() {
    check("text_and_integer");
}

#[test]
fn group() {
    check("group");
}

#[test]
fn choice_filter() {
    check("choice_filter_test");
}

#[test]
fn repeat_with_count_and_generated_notes() {
    check("repeat_date_test");
}

#[test]
fn widgets_with_table_lists() {
    check("widgets");
}

#[test]
fn multilanguage_spec_test() {
    check("xlsform_spec_test");
}

#[test]
fn or_other() {
    check("or_other");
    check("or_other_multi");
}

#[test]
fn pulldata_and_external_instances() {
    check("pull_data");
    check("externals");
}

#[test]
fn dynamic_defaults_become_setvalue_actions() {
    check("dynamic_default");
    check("default_in_repeat");
}

#[test]
fn trigger_column_and_odk_actions() {
    check("trigger_col");
    check("actions_geo");
    check("bg_geopoint");
}

#[test]
fn select_from_file_and_randomize() {
    check("select_from_file");
    check("randomize");
}

#[test]
fn capture_parameters_become_attributes() {
    check("params_media");
}

#[test]
fn audit_lives_in_meta_with_odk_parameters() {
    check("audit_params");
}

#[test]
fn generic_column_prefixes_pass_through() {
    check("instance_attr");
    check("settings_extra");
}

#[test]
fn duplicate_choices_allowed_by_setting() {
    check("dup_choices_allowed");
}

#[test]
fn entities_create_update_forms() {
    check("entities_create");
    check("entities_update");
    check("entities_create_update");
}

#[test]
fn search_appearance_inlines_items() {
    check("search_appearance");
}

#[test]
fn last_saved_references() {
    check("last_saved");
}

#[test]
fn sms_compact_representation() {
    check("sms_compact");
}

#[test]
fn clean_text_values_default_on_and_off() {
    check("clean_text");
    check("clean_text_off");
}

#[test]
fn flat_forms() {
    check("flat_xlsform_test");
}

#[test]
fn loop_over_choices() {
    check("loop");
}

#[test]
fn select_one_external_with_itemsets_csv() {
    check("sel_external");
    let wb = rxform::xls::read_workbook(Path::new("tests/fixtures/sel_external.xlsx")).unwrap();
    let conversion = rxform::convert_workbook_full(&wb, "sel_external").unwrap();
    let expected = std::fs::read_to_string("tests/expected/sel_external_itemsets.csv").unwrap();
    assert_eq!(conversion.itemsets_csv.as_deref(), Some(expected.as_str()));
}

#[test]
fn unknown_type_is_an_error() {
    let wb = rxform::xls::Workbook {
        survey: rxform::xls::sheet_from_rows(vec![
            vec!["type".into(), "name".into()],
            vec!["no_such_type".into(), "q1".into()],
        ]),
        ..Default::default()
    };
    let err = rxform::convert_workbook(&wb, "t").unwrap_err();
    assert!(err.to_string().contains("unknown question type"), "{err}");
}

#[test]
fn missing_choice_list_is_an_error() {
    let wb = rxform::xls::Workbook {
        survey: rxform::xls::sheet_from_rows(vec![
            vec!["type".into(), "name".into()],
            vec!["select_one ghosts".into(), "q1".into()],
        ]),
        ..Default::default()
    };
    let err = rxform::convert_workbook(&wb, "t").unwrap_err();
    assert!(err.to_string().contains("ghosts"), "{err}");
}

#[test]
fn unresolved_reference_is_an_error() {
    let wb = rxform::xls::Workbook {
        survey: rxform::xls::sheet_from_rows(vec![
            vec!["type".into(), "name".into(), "relevant".into()],
            vec!["text".into(), "q1".into(), "${nope} = 1".into()],
        ]),
        ..Default::default()
    };
    let err = rxform::convert_workbook(&wb, "t").unwrap_err();
    assert!(err.to_string().contains("nope"), "{err}");
}
