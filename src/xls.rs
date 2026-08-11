//! Workbook reading: turns an .xlsx/.xls/.ods file into generic sheet data.

use std::collections::BTreeMap;
use std::path::Path;

use calamine::{open_workbook_auto, Data, Range, Reader};

use crate::error::{Error, Result};

/// One sheet as a list of rows. Each row keeps its 1-based spreadsheet row
/// number (for error messages) and maps trimmed header → trimmed cell value.
/// Empty cells are omitted.
#[derive(Debug, Default, Clone)]
pub struct Sheet {
    pub headers: Vec<String>,
    pub rows: Vec<(usize, BTreeMap<String, String>)>,
}

#[derive(Debug, Default, Clone)]
pub struct Workbook {
    pub survey: Sheet,
    pub choices: Sheet,
    pub settings: Sheet,
    pub external_choices: Sheet,
    pub entities: Sheet,
}

pub fn read_workbook(path: &Path) -> Result<Workbook> {
    let mut wb = open_workbook_auto(path).map_err(|e| {
        Error::new(format!("cannot open workbook: {e}"))
            .hint("supported formats are .xlsx, .xls and .ods")
    })?;
    let sheet_names = wb.sheet_names().to_vec();

    let find = |target: &str| -> Option<String> {
        sheet_names
            .iter()
            .find(|n| n.trim().eq_ignore_ascii_case(target))
            .cloned()
    };

    let survey_name =
        find("survey").ok_or_else(|| Error::new("the workbook has no 'survey' sheet").hint("an XLSForm needs a sheet named 'survey' (case-insensitive) with type/name/label columns"))?;
    let survey = read_sheet(&mut wb, &survey_name)?;

    let choices = match find("choices").or_else(|| find("choices and columns")) {
        Some(name) => read_sheet(&mut wb, &name)?,
        None => Sheet::default(),
    };
    let settings = match find("settings") {
        Some(name) => read_sheet(&mut wb, &name)?,
        None => Sheet::default(),
    };
    let external_choices = match find("external_choices") {
        Some(name) => read_sheet(&mut wb, &name)?,
        None => Sheet::default(),
    };
    let entities = match find("entities") {
        Some(name) => read_sheet(&mut wb, &name)?,
        None => Sheet::default(),
    };

    Ok(Workbook {
        survey,
        choices,
        settings,
        external_choices,
        entities,
    })
}

fn read_sheet(
    wb: &mut calamine::Sheets<std::io::BufReader<std::fs::File>>,
    name: &str,
) -> Result<Sheet> {
    let range: Range<Data> = wb
        .worksheet_range(name)
        .map_err(|e| Error::new(format!("cannot read sheet '{name}': {e}")).sheet(name))?;
    Ok(sheet_from_rows(range.rows().map(|r| {
        r.iter().map(cell_to_string).collect::<Vec<String>>()
    })))
}

/// Build a [`Sheet`] from raw rows of strings; the first non-empty row is the
/// header row. Public so tests can construct sheets without a real workbook.
pub fn sheet_from_rows<I>(rows: I) -> Sheet
where
    I: IntoIterator<Item = Vec<String>>,
{
    let mut headers: Option<Vec<String>> = None;
    let mut sheet = Sheet::default();
    for (i, row) in rows.into_iter().enumerate() {
        let row_number = i + 1;
        if row.iter().all(|c| c.trim().is_empty()) {
            continue;
        }
        match &headers {
            None => {
                // headers are trimmed; cell values keep their whitespace
                // (pyxform preserves spacing inside labels)
                let cells: Vec<String> = row.iter().map(|c| c.trim().to_string()).collect();
                sheet.headers = cells.iter().filter(|h| !h.is_empty()).cloned().collect();
                headers = Some(cells);
            }
            Some(hs) => {
                let mut map = BTreeMap::new();
                for (h, v) in hs.iter().zip(row.iter()) {
                    if !h.is_empty() && !v.trim().is_empty() {
                        map.insert(h.clone(), straighten_quotes(v));
                    }
                }
                if !map.is_empty() {
                    sheet.rows.push((row_number, map));
                }
            }
        }
    }
    sheet
}

/// pyxform replaces "smart" quotes with their plain ASCII forms in all cell
/// values, so expressions typed in word processors still parse.
fn straighten_quotes(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '\u{2018}' | '\u{2019}' | '\u{201A}' | '\u{201B}' => '\'',
            '\u{201C}' | '\u{201D}' | '\u{201E}' | '\u{201F}' => '"',
            other => other,
        })
        .collect()
}

/// Convert a cell to the string pyxform would see: integral floats lose the
/// trailing `.0`, booleans become TRUE/FALSE, dates render as ISO strings.
fn cell_to_string(cell: &Data) -> String {
    match cell {
        Data::Empty => String::new(),
        Data::String(s) => s.clone(),
        Data::Float(f) => {
            if f.fract() == 0.0 && f.abs() < 1e15 {
                format!("{}", *f as i64)
            } else {
                format!("{f}")
            }
        }
        Data::Int(i) => format!("{i}"),
        Data::Bool(b) => if *b { "TRUE" } else { "FALSE" }.to_string(),
        // pyxform sees openpyxl datetime objects, whose str() form is
        // "YYYY-MM-DD HH:MM:SS" even for date-only cells.
        Data::DateTime(dt) => match dt.as_datetime() {
            Some(ndt) => ndt.format("%Y-%m-%d %H:%M:%S").to_string(),
            None => format!("{}", dt.as_f64()),
        },
        Data::DateTimeIso(s) => s.clone(),
        Data::DurationIso(s) => s.clone(),
        Data::Error(e) => format!("{e:?}"),
    }
}
