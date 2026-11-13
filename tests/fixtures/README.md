# Test fixtures: provenance

The forms in this directory are the inputs of `tests/fixtures_test.rs`; the
matching outputs are in `tests/expected/`, validated against pyxform 4.5.0.

## Copied from pyxform

These nine files are byte-identical copies of files in the test suite of
[pyxform](https://github.com/XLSForm/pyxform), version 4.5.0, directory
`tests/example_xls/`:

- `choice_filter_test.xlsx`
- `flat_xlsform_test.xlsx`
- `group.xlsx`
- `loop.xlsx`
- `or_other.xlsx`
- `pull_data.xlsx`
- `text_and_integer.xlsx`
- `xlsform_spec_test.xlsx`
- `yes_or_no_question.xlsx`

`repeat_date_test.xlsx` and `widgets.xlsx` have the names of
`tests/example_xls/repeat_date_test.xls` and `tests/example_xls/widgets.xls`
in pyxform 4.5.0, in `.xlsx` format instead of `.xls`.

pyxform is Copyright (c) 2015, XLSForm, and distributed under the BSD
2-Clause license; its notice is reproduced verbatim in the
[`NOTICE`](../../NOTICE) file at the root of this repository.

## Prepared for rxform

The remaining files do not appear in pyxform 4.5.0, by name or by content.
They are small forms prepared for rxform, one per feature, to exercise
behaviour that pyxform implements (actions, audit parameters,
entities, external instances, `last-saved`, `or_other`, randomization,
`select_one_from_file`, settings, triggers and so on). They are covered by
rxform's own licence (BSD 2-Clause, see [`LICENSE`](../../LICENSE)).
