//! Golden test against a real AEMO DispatchIS file (see tests/data/README.md).

use nem_lens_aemo::files::{dispatch_file_interval, unzip_csvs};
use nem_lens_aemo::{dispatch_snapshot, parse_report};

const NAME: &str = "PUBLIC_DISPATCHIS_202610171205_0000000543344246.zip";
const ZIP: &[u8] = include_bytes!("data/PUBLIC_DISPATCHIS_202610171205_0000000543344246.zip");

#[test]
fn parses_real_dispatchis_report() {
    let members = unzip_csvs(ZIP).unwrap();
    assert_eq!(members.len(), 1);
    let tables = parse_report(&members[0].1).unwrap();
    for t in [
        "DISPATCH_CASE_SOLUTION",
        "DISPATCH_PRICE",
        "DISPATCH_REGIONSUM",
        "DISPATCH_INTERCONNECTORRES",
        "DISPATCH_CONSTRAINT",
    ] {
        assert!(tables.contains_key(t), "missing table {t}");
    }

    let snap = dispatch_snapshot(&tables).unwrap();
    assert_eq!(Some(snap.settlement), dispatch_file_interval(NAME));

    let rrp: Vec<(&str, f64)> = snap
        .regions
        .iter()
        .map(|(r, v)| (r.as_str(), v.rrp))
        .collect();
    assert_eq!(
        rrp,
        [
            ("NSW1", 47.09185),
            ("QLD1", 46.5),
            ("SA1", -85.02),
            ("TAS1", -46.59586),
            ("VIC1", -44.58)
        ]
    );
    assert_eq!(snap.regions["NSW1"].demand, Some(4387.2));
    assert!(snap
        .regions
        .values()
        .all(|r| r.demand.is_some() && r.available_gen.is_some() && r.net_interchange.is_some()));

    assert!(snap.constraints.len() > 500, "{}", snap.constraints.len());
    assert!(snap.constraints.iter().all(|c| !c.id.is_empty()));
    let ics: Vec<&str> = snap.interconnectors.iter().map(|i| i.id.as_str()).collect();
    assert_eq!(
        ics,
        [
            "N-Q-MNSP1",
            "NSW1-QLD1",
            "NSW1-SA1",
            "T-V-MNSP1",
            "V-S-MNSP1",
            "V-SA",
            "VIC1-NSW1"
        ]
    );
}
