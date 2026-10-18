//! NEMWeb file names and archives. Listings may be HTML or plain text; names
//! are found anywhere in the text.

use std::io::{Cursor, Read};
use std::sync::OnceLock;

use anyhow::{Context, Result};
use chrono::{NaiveDate, NaiveDateTime};
use regex::Regex;

/// Default per-member limit for [`unzip_csvs`]. DispatchIS members are tens of
/// kilobytes; next-day offer files reach hundreds of megabytes.
pub const MAX_MEMBER_BYTES: u64 = 1024 * 1024 * 1024;

fn dispatch_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"PUBLIC_DISPATCHIS_(\d{12})_(\d{16})\.zip").expect("valid regex"))
}

fn daily_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(PUBLIC_[A-Z_]+?)_(\d{8})_(\d{16})\.zip").expect("valid regex"))
}

/// DispatchIS file names in a listing, sorted and deduplicated.
pub fn dispatch_files(listing: &str) -> Vec<String> {
    let mut v: Vec<String> = dispatch_re()
        .find_iter(listing)
        .map(|m| m.as_str().to_string())
        .collect();
    v.sort();
    v.dedup();
    v
}

/// The interval-end time encoded in a DispatchIS file name.
pub fn dispatch_file_interval(name: &str) -> Option<NaiveDateTime> {
    let caps = dispatch_re().captures(name)?;
    NaiveDateTime::parse_from_str(&caps[1], "%Y%m%d%H%M").ok()
}

/// Daily files (Next_Day_*, Bidmove_*) in a listing, as (file name, trading day).
pub fn daily_files(listing: &str) -> Vec<(String, NaiveDate)> {
    let mut v: Vec<(String, NaiveDate)> = daily_re()
        .captures_iter(listing)
        .filter_map(|c| {
            let d = NaiveDate::parse_from_str(&c[2], "%Y%m%d").ok()?;
            Some((c[0].to_string(), d))
        })
        .collect();
    v.sort();
    v.dedup();
    v
}

/// All CSV members of a zip, recursing one level into nested zips, with
/// [`MAX_MEMBER_BYTES`] per member.
pub fn unzip_csvs(bytes: &[u8]) -> Result<Vec<(String, Vec<u8>)>> {
    unzip_csvs_with_limit(bytes, MAX_MEMBER_BYTES)
}

/// As [`unzip_csvs`], failing if any member decompresses to more than
/// `max_member` bytes. The limit is enforced while reading, so a member that
/// under-declares its size cannot exceed it.
pub fn unzip_csvs_with_limit(bytes: &[u8], max_member: u64) -> Result<Vec<(String, Vec<u8>)>> {
    let mut out = Vec::new();
    visit_csvs(bytes, max_member, |name, r| {
        let mut buf = Vec::new();
        r.read_to_end(&mut buf)?;
        out.push((name.to_string(), buf));
        Ok(())
    })?;
    Ok(out)
}

/// Stream each CSV member of a zip to `f` as it decompresses, without
/// buffering it; pair with [`crate::report::visit_report`] for large files.
/// Nested zips (one level) are buffered. Reading more than `max_member` bytes
/// from a member is an error.
pub fn visit_csvs(
    bytes: &[u8],
    max_member: u64,
    mut f: impl FnMut(&str, &mut dyn Read) -> Result<()>,
) -> Result<()> {
    visit_into(bytes, 0, max_member, &mut f)
}

fn visit_into(
    bytes: &[u8],
    depth: u8,
    max_member: u64,
    f: &mut dyn FnMut(&str, &mut dyn Read) -> Result<()>,
) -> Result<()> {
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).context("not a zip archive")?;
    for i in 0..zip.len() {
        let file = zip.by_index(i)?;
        let name = file.name()?.to_string();
        anyhow::ensure!(file.size() <= max_member, "zip member {name} too large");
        let lower = name.to_ascii_lowercase();
        let mut limited = Limited {
            inner: file,
            left: max_member,
            name: name.clone(),
        };
        if lower.ends_with(".csv") {
            f(&name, &mut limited)?;
        } else if lower.ends_with(".zip") && depth == 0 {
            let mut buf = Vec::new();
            limited.read_to_end(&mut buf)?;
            visit_into(&buf, depth + 1, max_member, f)?;
        }
    }
    Ok(())
}

/// A reader that fails once more than `left` bytes have been read, so a member
/// that under-declares its size still cannot exceed the limit.
struct Limited<R> {
    inner: R,
    left: u64,
    name: String,
}

impl<R: Read> Read for Limited<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        if n as u64 > self.left {
            return Err(std::io::Error::other(format!(
                "zip member {} too large",
                self.name
            )));
        }
        self.left -= n as u64;
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    fn zip_of(members: &[(&str, &[u8])]) -> Vec<u8> {
        let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, data) in members {
            w.start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            w.write_all(data).unwrap();
        }
        w.finish().unwrap().into_inner()
    }

    #[test]
    fn finds_files_in_html_and_text_listings() {
        let html = r#"<A HREF="/Reports/Current/DispatchIS_Reports/PUBLIC_DISPATCHIS_202610170955_0000000543328677.zip">PUBLIC_DISPATCHIS_202610170955_0000000543328677.zip</A><br>
        Saturday, October 17, 2026 09:46 AM 21391 PUBLIC_DISPATCHIS_202610170950_0000000543327927.zip"#;
        let files = dispatch_files(html);
        assert_eq!(files.len(), 2);
        assert_eq!(
            files[0],
            "PUBLIC_DISPATCHIS_202610170950_0000000543327927.zip"
        );
        assert_eq!(
            dispatch_file_interval(&files[1]).unwrap().to_string(),
            "2026-10-17 09:55:00"
        );
    }

    #[test]
    fn finds_daily_files_including_sparse_offers() {
        // Names as listed on NEMWeb, 17 Oct 2026. Next_Day_Offer_Energy is now
        // empty; offers are published as Next_Day_Offer_Energy_SPARSE.
        let text = "PUBLIC_NEXT_DAY_DISPATCH_20261016_0000000543289388.zip \
                    PUBLIC_NEXT_DAY_OFFER_ENERGY_SPARSE_20261016_0000000543293471.zip \
                    PUBLIC_BIDMOVE_COMPLETE_20261016_0000000543293467.zip";
        let files = daily_files(text);
        assert_eq!(files.len(), 3);
        assert!(files.iter().all(|(_, d)| d.to_string() == "2026-10-16"));
        assert!(files
            .iter()
            .any(|(n, _)| n.starts_with("PUBLIC_NEXT_DAY_OFFER_ENERGY_SPARSE_")));
    }

    #[test]
    fn unzips_csvs_and_one_level_of_nesting() {
        let inner = zip_of(&[("b.CSV", b"inner")]);
        let outer = zip_of(&[("a.csv", b"outer"), ("readme.txt", b"x"), ("n.zip", &inner)]);
        let got = unzip_csvs(&outer).unwrap();
        let names: Vec<&str> = got.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["a.csv", "b.CSV"]);
        assert_eq!(got[1].1, b"inner");
    }

    #[test]
    fn member_limit_is_enforced() {
        let z = zip_of(&[("a.csv", &[b'x'; 100])]);
        assert!(unzip_csvs_with_limit(&z, 99).is_err());
        assert!(unzip_csvs_with_limit(&z, 100).is_ok());
        assert!(unzip_csvs(b"not a zip").is_err());
    }

    #[test]
    fn reader_limit_holds_even_if_the_declared_size_lies() {
        let mut r = Limited {
            inner: Cursor::new(vec![b'x'; 100]),
            left: 99,
            name: "a.csv".into(),
        };
        let err = std::io::copy(&mut r, &mut std::io::sink()).unwrap_err();
        assert!(err.to_string().contains("too large"));
        let mut r = Limited {
            inner: Cursor::new(vec![b'x'; 100]),
            left: 100,
            name: "a.csv".into(),
        };
        assert_eq!(std::io::copy(&mut r, &mut std::io::sink()).unwrap(), 100);
    }

    #[test]
    fn visit_streams_members_into_the_report_visitor() {
        let csv = b"I,A,X,1,ID\nD,A,X,1,a\nD,A,X,1,b\n";
        let z = zip_of(&[("r.CSV", csv), ("skip.txt", b"x")]);
        let mut ids = Vec::new();
        visit_csvs(&z, MAX_MEMBER_BYTES, |name, r| {
            assert_eq!(name, "r.CSV");
            crate::report::visit_report(r, |rec| {
                ids.push(rec.text("ID").unwrap_or_default().to_string());
                Ok(())
            })
        })
        .unwrap();
        assert_eq!(ids, ["a", "b"]);
    }
}
