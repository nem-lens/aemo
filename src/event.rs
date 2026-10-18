//! The NEM Lens event record: one schema for everything kept about an
//! event, with every file and field tagged public or private.
//!
//! The public event library (`nem-lens/library`, the replay dashboard) is a
//! view of the record: only what is directly derived from AEMO's published
//! figures, for real events. The full record adds the AEMO tables as
//! fetched, kept in the private store that cf-report reads.
//! [`check_public_file`] is the gate the publisher runs on every file before
//! it reaches the library.
//!
//! Nothing the engine computes is stored anywhere (owner, 19 Oct 2026): the
//! replay, counterfactuals and attribution are computed on the user's own
//! machine, in WebAssembly, from the stored record.
//!
//! ```text
//! events/<id>/headline.json          public   AEMO headline (private fields stripped)
//! events/<id>/manifest.json          public   AEMO sources and SHA-256
//! events/<id>/series.json            public   regional price, demand, availability per interval
//! events/<id>/dispatch.json          public   dispatch by fuel in the event region
//! events/<id>/interconnectors.json   public   flows, AEMO limits and the constraints that set them
//! events/<id>/constraints.json       public   binding constraints, AEMO marginal values and descriptions
//! events/<id>/unavailability.json    public   units out or derated, with rebid explanations
//! events/<id>/notices.json           public   AEMO market notices in the window
//! events/<id>/bundle/...             private  AEMO tables as fetched and market notices (engine inputs, not displayed)
//! ```
//!
//! The headline's engine fields are computed, never stored; they are private
//! pending the owner's long-term decision on a third public page for the
//! model baseline, and making them public is a change of tag here.

use std::collections::BTreeMap;

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

/// Event schema version. Version 1 was the library's headline-only layout
/// with replay figures in the headline.
pub const SCHEMA: u32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Visibility {
    /// In the public library: directly derived from AEMO's published figures.
    Public,
    /// Only in the private store: engine output, counterfactuals, analysis,
    /// or inputs kept for them.
    Private,
}

/// One file (or folder, when `path` ends in `/`) of an event record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileSpec {
    pub path: &'static str,
    pub visibility: Visibility,
    /// What writes it.
    pub producer: &'static str,
    pub contents: &'static str,
}

use Visibility::{Private, Public};

/// Every file an event record may hold, relative to `events/<id>/`.
pub const FILES: &[FileSpec] = &[
    FileSpec {
        path: "headline.json",
        visibility: Public,
        producer: "monitor",
        contents: "AEMO headline figures; private fields stripped in the library",
    },
    FileSpec {
        path: "manifest.json",
        visibility: Public,
        producer: "monitor",
        contents: "AEMO source files, URLs and SHA-256",
    },
    FileSpec {
        path: "series.json",
        visibility: Public,
        producer: "monitor",
        contents: "per interval and region: price, demand, available generation, net interchange",
    },
    FileSpec {
        path: "dispatch.json",
        visibility: Public,
        producer: "monitor",
        contents: "dispatch by fuel in the event region, per interval",
    },
    FileSpec {
        path: "interconnectors.json",
        visibility: Public,
        producer: "monitor",
        contents: "flows, AEMO's limits and the constraints that set them",
    },
    FileSpec {
        path: "constraints.json",
        visibility: Public,
        producer: "monitor",
        contents: "binding generic constraints with AEMO's marginal values and descriptions",
    },
    FileSpec {
        path: "unavailability.json",
        visibility: Public,
        producer: "monitor",
        contents: "units on outage or derated, with AEMO rebid explanations",
    },
    FileSpec {
        path: "notices.json",
        visibility: Public,
        producer: "monitor",
        contents: "AEMO market notices issued in the window",
    },
    FileSpec {
        path: "bundle/",
        visibility: Private,
        producer: "monitor",
        contents: "AEMO tables as fetched (Parquet) and market notices, engine inputs",
    },
];

/// The spec a record path falls under, if any.
pub fn spec_for(path: &str) -> Option<&'static FileSpec> {
    let path = path.trim_start_matches("./").replace('\\', "/");
    FILES.iter().find(|f| {
        if f.path.ends_with('/') {
            path.starts_with(f.path)
        } else {
            path == f.path
        }
    })
}

/// JSON keys that hold private data wherever they appear: a public file
/// containing one is refused.
pub const PRIVATE_KEYS: &[&str] = &[
    "baseline_mean_price",
    "scenarios",
    "mean_price_change",
    "baseline",
    "fidelity",
    "shapley",
    "shares",
    "attribution",
    "solutions",
    "snapshots",
];

/// The headline of an event. Private fields are `None` in the library.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Headline {
    pub schema: u32,
    pub event_id: String,
    pub region: String,
    pub trading_day: String,
    /// First and last interval end of the event span, market time with
    /// offset.
    pub start: String,
    pub end: String,
    pub duration_minutes: u32,
    /// Highest AEMO regional price in the span, $/MWh, and when.
    pub peak_price: f64,
    pub peak_at: String,
    /// Mean AEMO regional price over the span, $/MWh.
    pub mean_price: f64,
    /// Intervals in the span at or above $5,000/MWh.
    pub intervals_above_5000: u32,
    /// Sum of AEMO price x AEMO demand (TOTALDEMAND) over the span, $.
    pub cost_to_load: f64,
    pub max_demand_mw: f64,
    /// Private, computed (never stored): the engine's mean baseline price
    /// over the span, $/MWh.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baseline_mean_price: Option<f64>,
    /// Private, computed (never stored): each scenario's change in mean
    /// price against the baseline.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scenarios: Option<Vec<ScenarioChange>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScenarioChange {
    pub name: String,
    pub mean_price_change: f64,
}

impl Headline {
    /// The library's view: private fields removed.
    pub fn public_view(&self) -> Headline {
        Headline {
            baseline_mean_price: None,
            scenarios: None,
            ..self.clone()
        }
    }
}

/// `index.json`: every event's public headline, newest first.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Index {
    pub schema: u32,
    pub events: Vec<Headline>,
}

/// `series.json`: AEMO's regional figures per interval (DISPATCH PRICE and
/// REGIONSUM, pricing run).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Series {
    pub schema: u32,
    /// Interval ends, market time with offset.
    pub intervals: Vec<String>,
    pub regions: BTreeMap<String, RegionSeries>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RegionSeries {
    /// RRP, $/MWh.
    pub price: Vec<Option<f64>>,
    /// TOTALDEMAND, MW.
    pub demand: Vec<Option<f64>>,
    /// AVAILABLEGENERATION, MW.
    pub available_generation: Vec<Option<f64>>,
    /// NETINTERCHANGE (positive is export), MW.
    pub net_interchange: Vec<Option<f64>>,
}

/// `dispatch.json`: the event region's dispatch by fuel (DISPATCH
/// UNIT_SOLUTION TOTALCLEARED, fuel from MMSDM GENUNITS
/// CO2E_ENERGY_SOURCE via DUALLOC).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Dispatch {
    pub schema: u32,
    pub region: String,
    pub intervals: Vec<String>,
    /// Fuel to MW per interval; batteries net of charging.
    pub fuels: BTreeMap<String, Vec<f64>>,
}

/// `interconnectors.json` (DISPATCH INTERCONNECTORRES).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Interconnectors {
    pub schema: u32,
    pub intervals: Vec<String>,
    pub interconnectors: Vec<InterconnectorSeries>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct InterconnectorSeries {
    pub id: String,
    pub from_region: String,
    pub to_region: String,
    /// METEREDMWFLOW, MW (positive from `from_region`).
    pub flow: Vec<Option<f64>>,
    /// IMPORTLIMIT and EXPORTLIMIT, MW.
    pub import_limit: Vec<Option<f64>>,
    pub export_limit: Vec<Option<f64>>,
    /// IMPORTGENCONID and EXPORTGENCONID: the constraint setting each limit.
    pub import_set_by: Vec<Option<String>>,
    pub export_set_by: Vec<Option<String>>,
}

/// `constraints.json`: generic constraints with a non-zero AEMO marginal
/// value in the window (DISPATCH CONSTRAINT), described from GENCONDATA.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Constraints {
    pub schema: u32,
    pub intervals: Vec<String>,
    pub constraints: Vec<ConstraintSeries>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ConstraintSeries {
    pub id: String,
    /// AEMO's description, e.g. "Out = Canberra-Capital (6) line, ...".
    pub description: String,
    /// Whether AEMO describes it as applying for a network outage.
    pub outage: bool,
    /// MARGINALVALUE per interval, $/MWh (None when not invoked).
    pub marginal: Vec<Option<f64>>,
    /// Intervals with a non-zero marginal value.
    pub intervals_binding: u32,
}

/// `unavailability.json`: units whose AEMO availability fell short of their
/// capacity in the window.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Unavailability {
    pub schema: u32,
    pub intervals: Vec<String>,
    pub units: Vec<UnitAvailability>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct UnitAvailability {
    pub duid: String,
    pub region: String,
    pub fuel: Option<String>,
    /// MAXCAPACITY from DUDETAILSUMMARY or GENUNITS, MW.
    pub capacity_mw: f64,
    /// AVAILABILITY per interval (DISPATCH UNIT_SOLUTION), MW.
    pub availability: Vec<Option<f64>>,
    /// "outage" (zero availability throughout) or "derated".
    pub kind: String,
    /// Rebids in force in the window: offer time and AEMO's
    /// REBIDEXPLANATION text.
    pub rebids: Vec<Rebid>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Rebid {
    pub offer_time: String,
    pub explanation: String,
}

/// `notices.json`: AEMO market notices issued in the window.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Notices {
    pub schema: u32,
    pub notices: Vec<Notice>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Notice {
    pub id: String,
    /// Notice type, e.g. "RESERVE NOTICE" or "INTER-REGIONAL TRANSFER".
    pub kind: String,
    /// Creation time, market time with offset.
    pub issued: String,
    /// AEMO's external reference: a one-line title.
    pub title: String,
    /// The notice text.
    pub reason: String,
}

/// The fuel name used in public files for an AEMO CO2E_ENERGY_SOURCE
/// (MMSDM GENUNITS): coal, gas, liquid, hydro, wind, solar, battery,
/// bioenergy or other; `None` when blank.
pub fn fuel_of(energy_source: &str) -> Option<&'static str> {
    let s = energy_source.trim().to_ascii_lowercase();
    let has = |w: &str| s.contains(w);
    if s.is_empty() {
        return None;
    }
    Some(if has("battery") {
        "battery"
    } else if has("bio") || has("bagasse") || has("landfill") {
        "bioenergy"
    } else if has("gas") || has("methane") || has("ethane") {
        "gas"
    } else if has("coal") {
        "coal"
    } else if has("diesel") || has("kerosene") || has("oil") {
        "liquid"
    } else if has("hydro") || has("water") {
        "hydro"
    } else if has("wind") {
        "wind"
    } else if has("solar") {
        "solar"
    } else {
        "other"
    })
}

/// Whether an AEMO constraint description marks a network outage
/// ("Out = <element>") rather than system normal ("Out = Nil").
pub fn is_outage_description(description: &str) -> bool {
    let d = description.trim_start();
    let Some(rest) = d
        .strip_prefix("Out")
        .or_else(|| d.strip_prefix("OUT"))
        .or_else(|| d.strip_prefix("out"))
    else {
        return false;
    };
    let rest = rest
        .trim_start()
        .trim_start_matches(['=', ':'])
        .trim_start();
    !rest.is_empty() && !rest.to_ascii_lowercase().starts_with("nil")
}

/// Parse one AEMO market notice (NEMWeb `Market_Notice`,
/// `NEMITWEB1_MKTNOTICE_*.R<id>`), a plain-text report. Times are market
/// time.
pub fn parse_market_notice(text: &str) -> Option<Notice> {
    let field = |name: &str| {
        text.lines().find_map(|l| {
            let (k, v) = l.split_once(':')?;
            (k.trim() == name).then(|| v.trim().to_string())
        })
    };
    let id = field("Notice ID")?;
    let kind = field("Notice Type ID").unwrap_or_default();
    let title = field("External Reference").unwrap_or_default();
    // "Creation Date :     12/10/2026     11:24:11"
    let created = text
        .lines()
        .find(|l| l.trim_start().starts_with("Creation Date"))
        .and_then(|l| l.split_once(':'))
        .map(|(_, v)| v.split_whitespace().collect::<Vec<_>>())
        .and_then(|p| {
            let d = chrono::NaiveDate::parse_from_str(p.first()?, "%d/%m/%Y").ok()?;
            let t = chrono::NaiveTime::parse_from_str(p.get(1)?, "%H:%M:%S").ok()?;
            Some(crate::time::iso_market(d.and_time(t)))
        })?;
    let reason = text
        .split_once("Reason :")
        .map(|(_, r)| {
            r.lines()
                .take_while(|l| !l.starts_with("-----"))
                .collect::<Vec<_>>()
                .join("\n")
                .trim()
                .to_string()
        })
        .unwrap_or_default();
    Some(Notice {
        id,
        kind,
        issued: created,
        title,
        reason,
    })
}

/// Refuse a file the publisher is about to put in the public library: its
/// path must be a public file of the schema, and a JSON file must not
/// contain any [`PRIVATE_KEYS`] key at any depth.
pub fn check_public_file(path: &str, bytes: &[u8]) -> Result<()> {
    match spec_for(path) {
        Some(f) if f.visibility == Public => {}
        Some(_) => bail!("{path} is private in the event schema"),
        None => bail!("{path} is not a file of the event schema"),
    }
    if path.ends_with(".json") {
        let v: serde_json::Value =
            serde_json::from_slice(bytes).map_err(|e| anyhow::anyhow!("{path}: not JSON: {e}"))?;
        if let Some(k) = private_key(&v) {
            bail!("{path} contains the private field {k:?}");
        }
    }
    Ok(())
}

fn private_key(v: &serde_json::Value) -> Option<&'static str> {
    match v {
        serde_json::Value::Object(m) => m.iter().find_map(|(k, x)| {
            PRIVATE_KEYS
                .iter()
                .find(|p| **p == k)
                .copied()
                .or_else(|| private_key(x))
        }),
        serde_json::Value::Array(a) => a.iter().find_map(private_key),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headline() -> Headline {
        Headline {
            schema: SCHEMA,
            event_id: "202610121420-NSW1-high".into(),
            region: "NSW1".into(),
            trading_day: "2026-10-12".into(),
            start: "2026-10-12T14:20:00+10:00".into(),
            end: "2026-10-12T15:35:00+10:00".into(),
            duration_minutes: 80,
            peak_price: 23_107.0,
            peak_at: "2026-10-12T14:30:00+10:00".into(),
            mean_price: 8_965.81,
            intervals_above_5000: 11,
            cost_to_load: 99.4e6,
            max_demand_mw: 9_000.0,
            baseline_mean_price: Some(8_614.22),
            scenarios: Some(vec![ScenarioChange {
                name: "coal back".into(),
                mean_price_change: -8_851.58,
            }]),
        }
    }

    #[test]
    fn the_public_view_passes_the_gate_and_the_full_record_does_not() {
        let h = headline();
        let full = serde_json::to_vec(&h).unwrap();
        assert!(check_public_file("headline.json", &full).is_err());
        let public = serde_json::to_vec(&h.public_view()).unwrap();
        check_public_file("headline.json", &public).unwrap();
        let back: Headline = serde_json::from_slice(&public).unwrap();
        assert_eq!(back.baseline_mean_price, None);
        assert_eq!(back.peak_price, 23_107.0);
    }

    #[test]
    fn notices_parse() {
        let text = "\
-------------------------------------------------------------------
                           MARKET NOTICE
-------------------------------------------------------------------

From :              AEMO
To   :              NEMITWEB1
Creation Date :     12/10/2026     11:24:11

-------------------------------------------------------------------

Notice ID               :         145591
Notice Type ID          :         INTER-REGIONAL TRANSFER
Notice Type Description :         Inter-Regional Transfer limit variation
Issue Date              :         12/10/2026
External Reference      :         Inter-regional transfer limit variation - Bendigo to Shepparton 220kV line - VIC region - 12/10/2026

-------------------------------------------------------------------

Reason :

AEMO ELECTRICITY MARKET NOTICE

At   1107 hrs 12/10/2026 there was an unplanned outage of Bendigo to Shepparton 220kV line.

-------------------------------------------------------------------
END OF REPORT
";
        let n = parse_market_notice(text).unwrap();
        assert_eq!(n.id, "145591");
        assert_eq!(n.kind, "INTER-REGIONAL TRANSFER");
        assert_eq!(n.issued, "2026-10-12T11:24:11+10:00");
        assert!(n
            .title
            .starts_with("Inter-regional transfer limit variation"));
        assert!(n.reason.contains("unplanned outage of Bendigo"));
        assert!(!n.reason.contains("END OF REPORT"));
    }

    #[test]
    fn fuels_and_outage_descriptions() {
        assert_eq!(fuel_of("Black coal"), Some("coal"));
        assert_eq!(fuel_of("Coal seam methane"), Some("gas"));
        assert_eq!(fuel_of("Landfill biogas methane"), Some("bioenergy"));
        assert_eq!(fuel_of("Battery Storage"), Some("battery"));
        assert_eq!(fuel_of(""), None);
        assert!(is_outage_description("Out = Canberra-Capital (6) line"));
        assert!(!is_outage_description("Out= Nil, Limit Heywood flows"));
        assert!(!is_outage_description("Hard Ramping constraint"));
    }

    #[test]
    fn private_paths_and_unknown_files_are_refused() {
        assert!(check_public_file("model/baseline.parquet", b"").is_err());
        assert!(check_public_file("cf/attribution.json", b"{}").is_err());
        assert!(check_public_file("bundle/bids_dayoffer.parquet", b"").is_err());
        assert!(check_public_file("snapshots.json.gz", b"").is_err());
        check_public_file(
            "series.json",
            br#"{"schema":2,"intervals":[],"regions":{}}"#,
        )
        .unwrap();
    }

    #[test]
    fn private_keys_are_found_at_any_depth() {
        let nested = br#"{"schema":2,"units":[{"duid":"X","notes":{"fidelity":0.9}}]}"#;
        assert!(check_public_file("unavailability.json", nested).is_err());
    }

    #[test]
    fn every_public_struct_round_trips() {
        let s = Series {
            schema: SCHEMA,
            intervals: vec!["2026-10-12T14:20:00+10:00".into()],
            regions: BTreeMap::from([(
                "NSW1".to_string(),
                RegionSeries {
                    price: vec![Some(23_107.0)],
                    ..RegionSeries::default()
                },
            )]),
        };
        let bytes = serde_json::to_vec(&s).unwrap();
        check_public_file("series.json", &bytes).unwrap();
        assert_eq!(serde_json::from_slice::<Series>(&bytes).unwrap(), s);
    }

    #[test]
    fn every_path_has_one_spec() {
        for f in FILES {
            let probe = if f.path.ends_with('/') {
                format!("{}x", f.path)
            } else {
                f.path.to_string()
            };
            assert_eq!(spec_for(&probe).map(|s| s.path), Some(f.path));
        }
    }
}
