//! Parser for AEMO's multi-table CSV report format.
//!
//! Rows start with C (comment), I (header) or D (data). I and D rows carry the
//! report name, table name and version in columns 1 to 3; fields follow from
//! column 4. Columns are looked up by name so version changes that add or
//! reorder fields do not break parsing.
//!
//! [`parse_report`] collects every table in memory. For large files (a
//! next-day offer file is about 250 MB of CSV) use [`visit_report`] or
//! [`parse_report_where`], which stream rows from any reader.
//!
//! The typed parsers ([`crate::bids`], [`crate::dispatch`]) read rows
//! through [`Fields`], so the same rules apply to a streamed [`Record`], a
//! parsed [`Table`] row, or a row of typed columns held elsewhere (an event
//! bundle's Parquet).

use std::borrow::Cow;
use std::collections::HashMap;
use std::io::Read;

use anyhow::{Context, Result};
use chrono::NaiveDateTime;

use crate::time::parse_market;

/// Named fields of one row in a table keyed like "BIDS_BIDDAYOFFER".
pub trait Fields {
    /// The table key, e.g. "DISPATCH_PRICE".
    fn key(&self) -> &str;

    /// A trimmed, non-empty field as text, or `None` if the column is
    /// absent or the field empty.
    fn text(&self, name: &str) -> Option<Cow<'_, str>>;

    /// A field as a number.
    fn num(&self, name: &str) -> Option<f64> {
        self.text(name).and_then(|s| s.parse().ok())
    }

    /// A field as a market time.
    fn time(&self, name: &str) -> Option<NaiveDateTime> {
        self.text(name).and_then(|s| parse_market(&s))
    }
}

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

    /// Each row as [`Fields`], under the table key `key`.
    pub fn records<'a>(&'a self, key: &'a str) -> impl Iterator<Item = TableRow<'a>> {
        self.rows.iter().map(move |row| TableRow {
            key,
            table: self,
            row,
        })
    }
}

/// One row of a [`Table`], as [`Fields`].
pub struct TableRow<'a> {
    pub key: &'a str,
    pub table: &'a Table,
    pub row: &'a [String],
}

impl Fields for TableRow<'_> {
    fn key(&self) -> &str {
        self.key
    }

    fn text(&self, name: &str) -> Option<Cow<'_, str>> {
        self.table
            .col(self.row, name)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(Cow::Borrowed)
    }
}

/// A table's columns from its I row, with a name index for fast lookup.
#[derive(Debug, Clone, Default)]
pub struct Header {
    pub columns: Vec<String>,
    index: HashMap<String, usize>,
}

impl Header {
    fn new(columns: Vec<String>) -> Self {
        let index = columns
            .iter()
            .enumerate()
            .map(|(i, c)| (c.clone(), i))
            .collect();
        Self { columns, index }
    }

    pub fn idx(&self, name: &str) -> Option<usize> {
        self.index.get(name).copied()
    }
}

/// One D row, borrowed while it streams past.
pub struct Record<'a> {
    /// Table key, e.g. "BIDS_BIDOFFERPERIOD_SPARSE".
    pub key: &'a str,
    pub header: &'a Header,
    fields: &'a csv::StringRecord,
}

impl<'a> Record<'a> {
    /// A field by column name (untrimmed), or `None` if the column is absent.
    pub fn get(&self, name: &str) -> Option<&'a str> {
        self.header.idx(name).and_then(|i| self.fields.get(i + 4))
    }

    /// A trimmed, non-empty field.
    pub fn text(&self, name: &str) -> Option<&'a str> {
        self.get(name).map(str::trim).filter(|s| !s.is_empty())
    }

    pub fn num(&self, name: &str) -> Option<f64> {
        self.text(name).and_then(|s| s.parse().ok())
    }

    /// Field values aligned with `header.columns`.
    pub fn values(&self) -> impl Iterator<Item = &'a str> {
        self.fields.iter().skip(4)
    }
}

impl Fields for Record<'_> {
    fn key(&self) -> &str {
        self.key
    }

    fn text(&self, name: &str) -> Option<Cow<'_, str>> {
        Record::text(self, name).map(Cow::Borrowed)
    }
}

fn table_key(rec: &csv::StringRecord) -> String {
    format!(
        "{}_{}",
        rec[1].trim().to_uppercase(),
        rec[2].trim().to_uppercase()
    )
}

enum Event<'a> {
    Header(&'a str, &'a Header),
    Data(&'a Record<'a>),
}

/// Stream a report: each I row, then each D row whose header has been seen.
/// D rows that arrive before their I row are dropped.
fn scan<R: Read>(reader: R, mut on: impl FnMut(Event) -> Result<()>) -> Result<()> {
    let mut rdr = csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .from_reader(reader);
    let mut headers: HashMap<String, Header> = HashMap::new();
    let mut rec = csv::StringRecord::new();
    while rdr.read_record(&mut rec).context("malformed CSV row")? {
        if rec.len() < 4 {
            continue;
        }
        match rec[0].trim() {
            "I" => {
                let key = table_key(&rec);
                let header = Header::new(
                    rec.iter()
                        .skip(4)
                        .map(|c| c.trim().to_uppercase())
                        .collect(),
                );
                on(Event::Header(&key, &header))?;
                headers.insert(key, header);
            }
            "D" => {
                let key = table_key(&rec);
                if let Some((key, header)) = headers.get_key_value(&key) {
                    on(Event::Data(&Record {
                        key,
                        header,
                        fields: &rec,
                    }))?;
                }
            }
            _ => {}
        }
    }
    Ok(())
}

/// Call `f` for every D row, in file order, without collecting anything.
pub fn visit_report<R: Read>(reader: R, mut f: impl FnMut(&Record) -> Result<()>) -> Result<()> {
    scan(reader, |e| match e {
        Event::Header(..) => Ok(()),
        Event::Data(r) => f(r),
    })
}

/// Collect the rows for which `keep` returns true. Every table seen gets an
/// entry with its columns, even if no rows are kept.
pub fn parse_report_where<R: Read>(
    reader: R,
    mut keep: impl FnMut(&Record) -> bool,
) -> Result<Tables> {
    let mut tables = Tables::new();
    scan(reader, |e| {
        match e {
            Event::Header(key, header) => {
                tables.entry(key.to_string()).or_default().columns = header.columns.clone();
            }
            Event::Data(r) => {
                if keep(r) {
                    if let Some(t) = tables.get_mut(r.key) {
                        t.rows.push(r.values().map(str::to_string).collect());
                    }
                }
            }
        }
        Ok(())
    })?;
    Ok(tables)
}

/// Parse one report into all of its tables.
pub fn parse_report(bytes: &[u8]) -> Result<Tables> {
    parse_report_where(bytes, |_| true)
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

    const TWO_TABLES: &str = "I,A,X,1,ID,V\nD,A,X,1,a, 1.5 \nD,A,X,1,b,\nI,A,Y,1,ID\nD,A,Y,1,c\n";

    #[test]
    fn visit_streams_rows_with_named_access() {
        let mut seen = Vec::new();
        visit_report(TWO_TABLES.as_bytes(), |r| {
            let own = |s: Option<&str>| s.map(str::to_string);
            seen.push((
                r.key.to_string(),
                own(r.text("ID")),
                r.num("V"),
                own(r.get("V")),
            ));
            Ok(())
        })
        .unwrap();
        let s = |v: &str| Some(v.to_string());
        assert_eq!(
            seen,
            [
                ("A_X".into(), s("a"), Some(1.5), s(" 1.5 ")),
                ("A_X".into(), s("b"), None, s("")),
                ("A_Y".into(), s("c"), None, None),
            ]
        );
    }

    #[test]
    fn visitor_errors_stop_the_scan() {
        let mut n = 0;
        let err = visit_report(TWO_TABLES.as_bytes(), |_| {
            n += 1;
            anyhow::bail!("stop")
        });
        assert!(err.is_err());
        assert_eq!(n, 1);
    }

    #[test]
    fn parse_where_keeps_selected_rows_and_all_headers() {
        let tables =
            parse_report_where(TWO_TABLES.as_bytes(), |r| r.text("ID") == Some("b")).unwrap();
        assert_eq!(tables["A_X"].rows, [vec!["b".to_string(), String::new()]]);
        assert!(tables["A_Y"].rows.is_empty());
        assert_eq!(tables["A_Y"].columns, ["ID"]);
    }
}
