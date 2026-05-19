//! Parser for `tmux list-panes -F '#{pane_id} #{pane_left} #{pane_top} #{pane_width} #{pane_height}'`.
//!
//! Format-string syntax intentionally pinned: we want a fixed
//! whitespace-separated layout so we can parse without regex. Hosts
//! that want to use this crate should invoke tmux with the documented
//! format above and pipe stdout to [`parse_list_panes`].

use std::str::FromStr;

/// A single tmux pane after parsing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pane {
    /// `#{pane_id}`, including the leading `%`.
    pub id: PaneId,
    /// `#{pane_left}` — cells from the left edge of the terminal.
    pub left: u16,
    /// `#{pane_top}` — cells from the top edge of the terminal.
    pub top: u16,
    /// `#{pane_width}` — width in cells.
    pub width: u16,
    /// `#{pane_height}` — height in cells.
    pub height: u16,
}

/// Numeric pane id (the part of `%N` after the `%`).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PaneId(pub u32);

/// Parse-time errors.
#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    /// A line did not have the expected number of fields.
    #[error("expected 5 whitespace-separated fields, got {0}: {1:?}")]
    BadLine(usize, String),
    /// A numeric field could not be parsed.
    #[error("invalid number in field {field}: {value}")]
    BadNumber {
        /// Which field name failed.
        field: &'static str,
        /// The offending value.
        value: String,
    },
    /// A pane id did not start with `%`.
    #[error("pane id must start with '%': {0}")]
    BadPaneId(String),
}

/// Parse a multi-line `tmux list-panes -F` output.
pub fn parse_list_panes(input: &str) -> Result<Vec<Pane>, ParseError> {
    let mut out = Vec::new();
    for line in input.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        out.push(parse_line(trimmed)?);
    }
    Ok(out)
}

fn parse_line(line: &str) -> Result<Pane, ParseError> {
    let fields: Vec<&str> = line.split_whitespace().collect();
    if fields.len() != 5 {
        return Err(ParseError::BadLine(fields.len(), line.to_owned()));
    }
    let id = parse_id(fields[0])?;
    let left = parse_field("pane_left", fields[1])?;
    let top = parse_field("pane_top", fields[2])?;
    let width = parse_field("pane_width", fields[3])?;
    let height = parse_field("pane_height", fields[4])?;
    Ok(Pane {
        id,
        left,
        top,
        width,
        height,
    })
}

fn parse_id(raw: &str) -> Result<PaneId, ParseError> {
    let rest = raw
        .strip_prefix('%')
        .ok_or_else(|| ParseError::BadPaneId(raw.to_owned()))?;
    let n: u32 =
        FromStr::from_str(rest).map_err(|_| ParseError::BadPaneId(raw.to_owned()))?;
    Ok(PaneId(n))
}

fn parse_field<T: FromStr>(field: &'static str, raw: &str) -> Result<T, ParseError> {
    FromStr::from_str(raw).map_err(|_| ParseError::BadNumber {
        field,
        value: raw.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_two_panes() {
        let input = "%0 0 0 100 30\n%1 100 0 100 30\n";
        let panes = parse_list_panes(input).unwrap();
        assert_eq!(panes.len(), 2);
        assert_eq!(
            panes[0],
            Pane {
                id: PaneId(0),
                left: 0,
                top: 0,
                width: 100,
                height: 30,
            }
        );
        assert_eq!(panes[1].id, PaneId(1));
        assert_eq!(panes[1].left, 100);
    }

    #[test]
    fn rejects_bad_pane_id() {
        let err = parse_list_panes("0 0 0 100 30\n").unwrap_err();
        assert!(matches!(err, ParseError::BadPaneId(_)));
    }

    #[test]
    fn rejects_wrong_field_count() {
        let err = parse_list_panes("%0 0 0 100\n").unwrap_err();
        assert!(matches!(err, ParseError::BadLine(4, _)));
    }
}
