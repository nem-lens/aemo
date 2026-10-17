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
    unzip_into(bytes, &mut out, 0, max_member)?;
    Ok(out)
}

fn unzip_into(
    bytes: &[u8],
    out: &mut Vec<(String, Vec<u8>)>,
    depth: u8,
    max_member: u64,
) -> Result<()> {
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).context("not a zip archive")?;
    for i in 0..zip.len() {
        let f = zip.by_index(i)?;
        let name = f.name().to_string();
        anyhow::ensure!(f.size() <= max_member, "zip member {name} too large");
        let mut buf = Vec::with_capacity(f.size() as usize);
        f.take(max_member + 1).read_to_end(&mut buf)?;
        anyhow::ensure!(
            buf.len() as u64 <= max_member,
            "zip member {name} too large"
        );
        let lower = name.to_ascii_lowercase();
        if lower.ends_with(".csv") {
            out.push((name, buf));
        } else if lower.ends_with(".zip") && depth == 0 {
            unzip_into(&buf, out, depth + 1, max_member)?;
        }
    }
    Ok(())
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
}
