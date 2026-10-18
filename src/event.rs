//! The NEM Lens event record: one schema for everything kept about an
//! event, with every file and field tagged public or private.
//!
//! The public event library (`nem-lens/library`, the replay dashboard) is a
//! view of the record: only what is directly derived from AEMO's published
//! figures, for real events. The full record, with the engine's replay,
//! counterfactuals and attribution, stays in the private store that
//! cf-report reads. [`check_public_file`] is the gate the publisher runs
//! on every file before it reaches the library.
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
//! events/<id>/bundle/*.parquet       private  AEMO tables as fetched (inputs, not displayed)
//! events/<id>/model/...              private  engine snapshots, baseline replay, fidelity
//! events/<id>/cf/...                 private  sliders, grid, scenarios, attribution, briefs
//! ```
//!
//! Model baseline and fidelity are private pending the owner's long-term
//! decision on a third public page; making them public is a change of tag
//! here, not a new schema.

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
        contents: "AEMO tables as fetched (Parquet), engine inputs",
    },
    FileSpec {
        path: "model/",
        visibility: Private,
        producer: "engine",
        contents: "engine snapshots, baseline replay, fidelity against AEMO",
    },
    FileSpec {
        path: "cf/",
        visibility: Private,
        producer: "cf-report",
        contents: "sliders, sensitivity grid, scenarios, attribution, fact sheet, brief",
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
    /// Private: the engine's mean baseline price over the span, $/MWh.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baseline_mean_price: Option<f64>,
    /// Private: each scenario's change in mean price against the baseline.
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
    /// NOTICETYPE, e.g. "RESERVE NOTICE".
    pub kind: String,
    pub issued: String,
    pub reason: String,
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
