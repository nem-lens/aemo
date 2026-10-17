//! Parser for AEMO's multi-table CSV report format.
//!
//! Rows start with C (comment), I (header) or D (data). I and D rows carry the
//! report name, table name and version in columns 1 to 3; fields follow from
//! column 4. Columns are looked up by name so version changes that add or
//! reorder fields do not break parsing.

use std::collections::HashMap;

use anyhow::{Context, Result};

#[derive(Debug, Default, Clone)]
pub struct Table {
    /// Column names, upper-cased, without the four leading I-row fields.
    pub columns: Vec<String>,
    /// Data rows, aligned with `columns`.
    pub rows: Vec<Vec<String>>,
}

/// Tables in one report, keyed "REPORT_TABLE", e.g. "DISPATCH_PRICE".
pub type Tables = HashMap<String, Table>;

impl Table {
    pub fn idx(&self, name: &str) -> Option<usize> {
        self.columns.iter().position(|c| c == name)
    }

    pub fn col<'a>(&self, row: &'a [String], name: &str) -> Option<&'a str> {
        self.idx(name).and_then(|i| row.get(i)).map(|s| s.as_str())
    }

    pub fn num(&self, row: &[String], name: &str) -> Option<f64> {
        self.col(row, name)
            .and_then(|s| s.trim().parse::<f64>().ok())
    }
}

/// Parse one report into its tables. D rows that arrive before their I row are dropped.
pub fn parse_report(bytes: &[u8]) -> Result<Tables> {
    let mut rdr = csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .from_reader(bytes);
    let mut tables = Tables::new();
    for rec in rdr.records() {
        let rec = rec.context("malformed CSV row")?;
        if rec.len() < 4 {
            continue;
        }
        let key = format!(
            "{}_{}",
            rec[1].trim().to_uppercase(),
            rec[2].trim().to_uppercase()
        );
        match rec[0].trim() {
            "I" => {
                let t = tables.entry(key).or_default();
                t.columns = rec
                    .iter()
                    .skip(4)
                    .map(|c| c.trim().to_uppercase())
                    .collect();
            }
            "D" => {
                if let Some(t) = tables.get_mut(&key) {
                    t.rows
                        .push(rec.iter().skip(4).map(|c| c.to_string()).collect());
                }
            }
            _ => {}
        }
    }
    Ok(tables)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn columns_are_found_by_name_in_any_order() {
        let csv = "I,DISPATCH,PRICE,6,RRP,EXTRA,REGIONID\nD,DISPATCH,PRICE,6,99.5,x,NSW1\n";
        let t = &parse_report(csv.as_bytes()).unwrap()["DISPATCH_PRICE"];
        assert_eq!(t.num(&t.rows[0], "RRP"), Some(99.5));
        assert_eq!(t.col(&t.rows[0], "REGIONID"), Some("NSW1"));
        assert_eq!(t.col(&t.rows[0], "MISSING"), None);
    }

    #[test]
    fn data_before_header_and_short_rows_are_skipped() {
        let csv = "D,DISPATCH,PRICE,5,1\nC,x\nI,DISPATCH,PRICE,5,RRP\nD,DISPATCH,PRICE,5,2\n";
        let tables = parse_report(csv.as_bytes()).unwrap();
        assert_eq!(tables["DISPATCH_PRICE"].rows.len(), 1);
    }

    #[test]
    fn keys_are_upper_cased() {
        let csv = "I,dispatch,price,5,rrp\nD,dispatch,price,5,1\n";
        let tables = parse_report(csv.as_bytes()).unwrap();
        assert_eq!(tables["DISPATCH_PRICE"].idx("RRP"), Some(0));
    }
}
