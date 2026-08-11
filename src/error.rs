//! Error type carrying the spreadsheet location (sheet / row / column) and a
//! probable cause or fix suggestion, so form authors can find mistakes fast.

use std::fmt;

/// Where in the workbook the problem was found. Row numbers are 1-based
/// spreadsheet rows (the header row is row 1).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Location {
    pub sheet: Option<String>,
    pub row: Option<usize>,
    pub column: Option<String>,
}

impl fmt::Display for Location {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut parts: Vec<String> = Vec::new();
        if let Some(sheet) = &self.sheet {
            parts.push(format!("sheet '{sheet}'"));
        }
        if let Some(row) = self.row {
            parts.push(format!("row {row}"));
        }
        if let Some(column) = &self.column {
            parts.push(format!("column '{column}'"));
        }
        write!(f, "{}", parts.join(", "))
    }
}

#[derive(Debug)]
pub struct Error {
    pub message: String,
    pub location: Location,
    /// Probable cause or suggested fix, shown on its own line.
    pub hint: Option<String>,
    pub source: Option<std::io::Error>,
}

impl Error {
    pub fn new(message: impl Into<String>) -> Self {
        Error {
            message: message.into(),
            location: Location::default(),
            hint: None,
            source: None,
        }
    }

    pub fn sheet(mut self, sheet: impl Into<String>) -> Self {
        self.location.sheet = Some(sheet.into());
        self
    }

    pub fn row(mut self, row: usize) -> Self {
        self.location.row = Some(row);
        self
    }

    pub fn column(mut self, column: impl Into<String>) -> Self {
        self.location.column = Some(column.into());
        self
    }

    pub fn hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    /// Attach a hint only when there is one to attach.
    pub fn maybe_hint(mut self, hint: Option<String>) -> Self {
        self.hint = hint;
        self
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.location == Location::default() {
            write!(f, "{}", self.message)?;
        } else {
            write!(f, "[{}] {}", self.location, self.message)?;
        }
        if let Some(hint) = &self.hint {
            write!(f, "\n  probable cause: {hint}")?;
        }
        Ok(())
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source.as_ref().map(|e| e as _)
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error {
            message: e.to_string(),
            location: Location::default(),
            hint: None,
            source: Some(e),
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// Levenshtein distance, used to suggest likely intended spellings.
pub(crate) fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut curr = vec![0; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        curr[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            curr[j + 1] = (prev[j] + cost).min(prev[j + 1] + 1).min(curr[j] + 1);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[b.len()]
}

/// The closest candidate to `input` within a sane distance, for "did you
/// mean ...?" hints.
pub(crate) fn closest<'a, I>(input: &str, candidates: I) -> Option<&'a str>
where
    I: IntoIterator<Item = &'a str>,
{
    let input_lower = input.to_lowercase();
    let max = (input.chars().count() / 3).max(2);
    candidates
        .into_iter()
        .map(|c| (edit_distance(&input_lower, &c.to_lowercase()), c))
        .filter(|(d, _)| *d <= max)
        .min_by_key(|(d, _)| *d)
        .map(|(_, c)| c)
}

/// A "did you mean 'x'?" hint if some candidate is close enough.
pub(crate) fn did_you_mean<'a, I>(input: &str, candidates: I) -> Option<String>
where
    I: IntoIterator<Item = &'a str>,
{
    closest(input, candidates).map(|c| format!("did you mean '{c}'?"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_location_and_hint() {
        let err = Error::new("unknown question type 'txet'")
            .sheet("survey")
            .row(3)
            .column("type")
            .hint("did you mean 'text'?");
        assert_eq!(
            err.to_string(),
            "[sheet 'survey', row 3, column 'type'] unknown question type 'txet'\n  probable cause: did you mean 'text'?"
        );
    }

    #[test]
    fn suggests_close_matches() {
        assert_eq!(
            closest("selct_one", ["select_one", "select_multiple"]),
            Some("select_one")
        );
        assert_eq!(closest("zzzz", ["select_one"]), None);
        assert_eq!(
            did_you_mean("integr", ["integer", "text"]).unwrap(),
            "did you mean 'integer'?"
        );
    }
}
