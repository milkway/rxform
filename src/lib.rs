//! rxform — a Rust implementation of [pyxform](https://github.com/xlsform/pyxform):
//! converts XLSForm spreadsheets into ODK XForm XML.
//!
//! ```no_run
//! let xml = rxform::convert_file(std::path::Path::new("survey.xlsx")).unwrap();
//! println!("{xml}");
//! ```

pub mod error;
pub mod model;
pub mod parser;
pub mod types;
pub mod xform;
pub mod xls;
pub mod xmlwriter;

pub use error::{Error, Result};

use std::path::Path;

/// A finished conversion: the XForm document plus the `itemsets.csv`
/// companion file when the workbook has an `external_choices` sheet.
pub struct Conversion {
    pub xml: String,
    pub itemsets_csv: Option<String>,
}

/// Convert an XLSForm workbook file (.xlsx/.xls/.ods) to XForm XML.
pub fn convert_file(path: &Path) -> Result<String> {
    Ok(convert(path)?.xml)
}

/// Convert an XLSForm workbook file, returning all conversion outputs.
pub fn convert(path: &Path) -> Result<Conversion> {
    let workbook = xls::read_workbook(path)?;
    let fallback = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("data")
        .to_string();
    convert_workbook_full(&workbook, &fallback)
}

/// Convert already-loaded sheet data to XForm XML. `fallback_name` supplies
/// the form id/name/title when the settings sheet does not define them.
pub fn convert_workbook(workbook: &xls::Workbook, fallback_name: &str) -> Result<String> {
    Ok(convert_workbook_full(workbook, fallback_name)?.xml)
}

/// Convert already-loaded sheet data, returning all conversion outputs.
pub fn convert_workbook_full(workbook: &xls::Workbook, fallback_name: &str) -> Result<Conversion> {
    let survey = parser::parse(workbook, fallback_name)?;
    let xml = xform::generate(&survey)?;
    Ok(Conversion {
        xml,
        itemsets_csv: xform::itemsets_csv(&survey),
    })
}
