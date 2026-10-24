//! The NEM Lens event record: one schema for everything kept about an
//! event, with every file and field tagged public or private.
//!
//! The public [event library](https://nem-lens.github.io/library/) is a view
//! of the record: only what is directly derived from AEMO's published
//! figures, for real events. The full record adds the AEMO tables as
//! fetched, kept in a private store. [`check_public_file`] is the gate the
//! publisher runs on every file before it reaches the library.
//!
//! Model output is never stored in a record; it is computed from the record
//! on the reader's own machine.
//!
//! ```text
//! events/<id>/headline.json          public   AEMO headline (private fields stripped)
//! events/<id>/manifest.json          public   AEMO sources and SHA-256
//! events/<id>/series.json            public   regional price, demand, availability, contingency reserve per interval
//! events/<id>/dispatch.json          public   dispatch by fuel in the event region
//! events/<id>/interconnectors.json   public   flows, AEMO limits and the constraints that set them
//! events/<id>/constraints.json       public   binding constraints, AEMO marginal values and descriptions
//! events/<id>/unavailability.json    public   units out or derated, with rebid explanations
//! events/<id>/notices.json           public   AEMO market notices in the window
//! events/<id>/bundle/...             private  AEMO tables as fetched and market notices (engine inputs, not displayed)
//! ```
//!
//! The headline's model fields ([`Headline::baseline_mean_price`] and
//! [`Headline::scenarios`]) are private; [`Headline::public_view`] strips
//! them.
//!
//! # Example
//!
//! ```
//! use nem_lens_aemo::event::check_public_file;
//!
//! let series = br#"{"schema":2,"intervals":[],"regions":{}}"#;
//! assert!(check_public_file("series.json", series).is_ok());
//!
//! // Private paths, unknown files and private fields are refused.
//! assert!(check_public_file("bundle/unit_dispatch.parquet", b"").is_err());
//! assert!(check_public_file("notes.json", b"{}").is_err());
//! let leaky = br#"{"schema":2,"notices":[],"baseline":{}}"#;
//! assert!(check_public_file("notices.json", leaky).is_err());
//! ```

use std::collections::BTreeMap;

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

/// Event schema version. Version 1 was the library's headline-only layout
/// with replay figures in the headline.
pub const SCHEMA: u32 = 2;

/// Where a file or field of the record may appear.
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
    /// Path relative to `events/<id>/`.
    pub path: &'static str,
    /// Public or private.
    pub visibility: Visibility,
    /// What writes it.
    pub producer: &'static str,
    /// What it holds, in a few words.
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
        contents: "per interval and region: price, demand, available generation, net interchange, contingency reserve ratio",
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
    /// [`SCHEMA`].
    pub schema: u32,
    /// The event's ID: start time, region and kind, e.g.
    /// "202610121420-NSW1-high".
    pub event_id: String,
    /// The event region's REGIONID.
    pub region: String,
    /// The trading day of the start, `YYYY-MM-DD`.
    pub trading_day: String,
    /// First interval end of the event span, market time with offset.
    pub start: String,
    /// Last interval end of the span.
    pub end: String,
    /// Five minutes per interval in the span.
    pub duration_minutes: u32,
    /// Highest AEMO regional price in the span, $/MWh.
    pub peak_price: f64,
    /// The interval end of the peak.
    pub peak_at: String,
    /// Mean AEMO regional price over the span, $/MWh.
    pub mean_price: f64,
    /// Intervals in the span at or above $5,000/MWh.
    pub intervals_above_5000: u32,
    /// Sum of AEMO price x AEMO demand (TOTALDEMAND) over the span, $.
    pub cost_to_load: f64,
    /// Highest TOTALDEMAND in the span, MW.
    pub max_demand_mw: f64,
    /// Facets for browsing, `key:value`, derived from AEMO data only (see
    /// [`TAG_KEYS`]). Absent in files written before tags existed.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Private, computed (never stored): the engine's mean baseline price
    /// over the span, $/MWh.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baseline_mean_price: Option<f64>,
    /// Private, computed (never stored): each scenario's change in mean
    /// price against the baseline.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scenarios: Option<Vec<ScenarioChange>>,
}

/// Private: one scenario's effect on the event's mean price.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScenarioChange {
    /// The scenario's name.
    pub name: String,
    /// Its mean price less the baseline's, $/MWh.
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
    /// [`SCHEMA`].
    pub schema: u32,
    /// Public headlines, newest first.
    pub events: Vec<Headline>,
}

/// `series.json`: AEMO's regional figures per interval (DISPATCH PRICE and
/// REGIONSUM, pricing run).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Series {
    /// [`SCHEMA`].
    pub schema: u32,
    /// Interval ends, market time with offset.
    pub intervals: Vec<String>,
    /// Every region by REGIONID.
    pub regions: BTreeMap<String, RegionSeries>,
}

/// One region's figures in [`Series`], each aligned with
/// [`Series::intervals`].
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
    /// Contingency reserve ratio (CRR): the region's largest single
    /// contingency, a unit (DUID, TOTALCLEARED) or a DC link (Basslink,
    /// Murraylink, Directlink: flow into the region), and the reserve left to
    /// replace it: unit headroom (AVAILABILITY − TOTALCLEARED) plus the room
    /// to raise imports to AEMO's interconnector limits, less the tripped
    /// element's own headroom. CRR = reserve / contingency; below 1, the
    /// region could not replace its largest loss. Arithmetic on AEMO figures
    /// (DISPATCH UNIT_SOLUTION and INTERCONNECTORRES, pricing run); absent
    /// in files written before it existed, or without next-day unit data.
    ///
    /// Inspired by WattClarity's Instantaneous Reserve Plant Margin (IRPM),
    /// surplus available generation over demand across the NEM. The CRR is
    /// per region and scaled by the largest contingency rather than demand;
    /// the library README sets out the differences and why.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub contingency: Vec<Option<String>>,
    /// The contingency's MW.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub contingency_mw: Vec<Option<f64>>,
    /// Reserve to replace it, MW.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reserve_mw: Vec<Option<f64>>,
    /// reserve_mw / contingency_mw.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub crr: Vec<Option<f64>>,
}

/// Whether an interconnector is a DC link (an MNSP: Basslink, Murraylink,
/// Directlink), which counts as a single contingency for the region it
/// feeds.
pub fn is_dc_link(id: &str) -> bool {
    id.contains("MNSP")
}

/// `dispatch.json`: the event region's dispatch by fuel (DISPATCH
/// UNIT_SOLUTION TOTALCLEARED, fuel from MMSDM GENUNITS
/// CO2E_ENERGY_SOURCE via DUALLOC).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Dispatch {
    /// [`SCHEMA`].
    pub schema: u32,
    /// The event region's REGIONID.
    pub region: String,
    /// Interval ends, market time with offset.
    pub intervals: Vec<String>,
    /// Fuel to MW per interval; batteries net of charging.
    pub fuels: BTreeMap<String, Vec<f64>>,
}

/// `interconnectors.json` (DISPATCH INTERCONNECTORRES).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Interconnectors {
    /// [`SCHEMA`].
    pub schema: u32,
    /// Interval ends, market time with offset.
    pub intervals: Vec<String>,
    /// Every interconnector.
    pub interconnectors: Vec<InterconnectorSeries>,
}

/// One interconnector in [`Interconnectors`], each series aligned with its
/// intervals.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct InterconnectorSeries {
    /// INTERCONNECTORID, e.g. "VIC1-NSW1".
    pub id: String,
    /// The region positive flow leaves.
    pub from_region: String,
    /// The region positive flow enters.
    pub to_region: String,
    /// MWFLOW (the dispatch target the limits apply to), MW, positive from
    /// `from_region`.
    pub flow: Vec<Option<f64>>,
    /// IMPORTLIMIT, MW, with AEMO's sign: the bound on `flow` in the
    /// negative direction. An import limit above zero forces flow from
    /// `from_region`.
    pub import_limit: Vec<Option<f64>>,
    /// EXPORTLIMIT, MW: the bound in the positive direction. An export
    /// limit below zero forces flow into `from_region`.
    pub export_limit: Vec<Option<f64>>,
    /// IMPORTGENCONID: the constraint setting the import limit.
    pub import_set_by: Vec<Option<String>>,
    /// EXPORTGENCONID: the constraint setting the export limit.
    pub export_set_by: Vec<Option<String>>,
}

/// `constraints.json`: generic constraints with a non-zero AEMO marginal
/// value or a violation in the window (DISPATCH CONSTRAINT), described from
/// GENCONDATA.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Constraints {
    /// [`SCHEMA`].
    pub schema: u32,
    /// Interval ends, market time with offset.
    pub intervals: Vec<String>,
    /// Constraints that bound or were violated.
    pub constraints: Vec<ConstraintSeries>,
}

/// One generic constraint in [`Constraints`], each series aligned with its
/// intervals.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ConstraintSeries {
    /// CONSTRAINTID.
    pub id: String,
    /// AEMO's description, e.g. "Out = Canberra-Capital (6) line, ...".
    pub description: String,
    /// Whether AEMO describes it as applying for a network outage.
    pub outage: bool,
    /// MARGINALVALUE per interval, $/MWh (None when not invoked).
    pub marginal: Vec<Option<f64>>,
    /// Intervals with a non-zero marginal value.
    pub intervals_binding: u32,
    /// VIOLATIONDEGREE per interval, in the constraint's units (MW for
    /// most): how far dispatch fell short of meeting it. None when not
    /// invoked or not violated. Absent in files written before it existed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub violation: Vec<Option<f64>>,
}

/// `unavailability.json`: units whose AEMO availability fell short of their
/// capacity in the window.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Unavailability {
    /// [`SCHEMA`].
    pub schema: u32,
    /// Interval ends, market time with offset.
    pub intervals: Vec<String>,
    /// Units out or derated.
    pub units: Vec<UnitAvailability>,
}

/// One unit in [`Unavailability`].
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct UnitAvailability {
    /// The dispatchable unit ID.
    pub duid: String,
    /// The unit's REGIONID.
    pub region: String,
    /// Its fuel, as [`fuel_of`] names it.
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

/// A rebid in force during an event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Rebid {
    /// When it was submitted, market time with offset.
    pub offer_time: String,
    /// AEMO's REBIDEXPLANATION, as the participant wrote it.
    pub explanation: String,
}

/// `notices.json`: AEMO market notices issued in the window.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Notices {
    /// [`SCHEMA`].
    pub schema: u32,
    /// Notices, as published.
    pub notices: Vec<Notice>,
}

/// One AEMO market notice.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Notice {
    /// The notice ID, e.g. "145591".
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

/// Tag keys on a headline, each `key:value`:
///
/// | Key | Values | From |
/// | --- | --- | --- |
/// | `year` | e.g. `2022` | event start, market time |
/// | `season` | `summer` (Dec-Feb), `autumn`, `winter`, `spring` | event start month |
/// | `region` | `NSW1`, `QLD1`, `VIC1`, `SA1`, `TAS1` | event region |
/// | `severity` | `cap` (peak at or above 95% of the price cap), `extreme` (at or above $10,000), `high` | peak price against [`market_price_cap`] |
/// | `price` | `at-cap` (peak at the cap), `negative` (a negative price in the span), `floor` (at the price floor) | DISPATCH PRICE RRP |
/// | `pricing` | `administered` (APCFLAG), `suspended` (MARKETSUSPENDEDFLAG) | DISPATCH PRICE, event region and span |
/// | `intervention` | `yes`: an intervention pricing run (direction or RERT) in the window | DISPATCH PRICE INTERVENTION = 1 rows |
/// | `settlement` | `5-minute` (from 1 Oct 2021, 04:00), `30-minute` | event start |
/// | `bids` | `missing` (no daily bids), `no-band-availability` (no per-period bids) | which bid tables the record holds |
pub const TAG_KEYS: &[&str] = &[
    "year",
    "season",
    "region",
    "severity",
    "price",
    "pricing",
    "intervention",
    "settlement",
    "bids",
];

/// The market price cap ($/MWh) in force at an interval end, by financial
/// year (AEMC Schedule of Reliability Settings). A financial year starts
/// with the interval ending 1 July 00:05.
pub fn market_price_cap(t: chrono::NaiveDateTime) -> f64 {
    use chrono::Datelike;
    let t = t - chrono::Duration::minutes(5);
    let fy = if t.month() >= 7 {
        t.year()
    } else {
        t.year() - 1
    };
    match fy {
        ..=2018 => 14_500.0,
        2019 => 14_700.0,
        2020 => 15_000.0,
        2021 => 15_100.0,
        2022 => 15_500.0,
        2023 => 16_600.0,
        2024 => 17_500.0,
        2025 => 20_300.0,
        _ => 23_200.0,
    }
}

/// The southern-hemisphere season of a month: summer is December to
/// February.
pub fn season(month: u32) -> &'static str {
    match month {
        12 | 1 | 2 => "summer",
        3..=5 => "autumn",
        6..=8 => "winter",
        _ => "spring",
    }
}

/// Five-minute settlement started with the trading day of 1 October 2021
/// (the interval ending 04:05).
pub fn five_minute_settlement(t: chrono::NaiveDateTime) -> bool {
    t > chrono::NaiveDate::from_ymd_opt(2021, 10, 1)
        .and_then(|d| d.and_hms_opt(4, 0, 0))
        .expect("valid date")
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

/// Regions at each end of a NEM interconnector, positive flow running
/// from the first to the second (MMSDM INTERCONNECTOR REGIONFROM and
/// REGIONTO).
pub fn interconnector_ends(id: &str) -> Option<(&'static str, &'static str)> {
    Some(match id {
        "N-Q-MNSP1" | "NSW1-QLD1" => ("NSW1", "QLD1"),
        "NSW1-SA1" => ("NSW1", "SA1"),
        "T-V-MNSP1" => ("TAS1", "VIC1"),
        "V-S-MNSP1" | "V-SA" => ("VIC1", "SA1"),
        "VIC1-NSW1" => ("VIC1", "NSW1"),
        _ => return None,
    })
}

/// Whether an AEMO constraint description marks a network outage
/// ("Out = *element*") rather than system normal ("Out = Nil").
///
/// ```
/// use nem_lens_aemo::event::is_outage_description;
///
/// assert!(is_outage_description("Out = Canberra-Capital (6) line"));
/// assert!(!is_outage_description("Out= Nil, Limit Heywood flows"));
/// ```
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
            tags: vec!["year:2026".into(), "season:spring".into()],
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
    fn caps_seasons_and_settlement() {
        let t = |s: &str| crate::time::parse_market(s).unwrap();
        assert_eq!(market_price_cap(t("2020/06/30 23:55:00")), 14_700.0);
        assert_eq!(market_price_cap(t("2020/07/01 00:05:00")), 15_000.0);
        assert_eq!(market_price_cap(t("2026/10/12 14:20:00")), 23_200.0);
        assert_eq!(season(12), "summer");
        assert_eq!(season(6), "winter");
        assert_eq!(season(9), "spring");
        assert!(!five_minute_settlement(t("2021/10/01 04:00:00")));
        assert!(five_minute_settlement(t("2021/10/01 04:05:00")));
        // Older headlines without tags still parse.
        let old = r#"{"schema":2,"event_id":"x","region":"NSW1","trading_day":"d","start":"s","end":"e",
            "duration_minutes":5,"peak_price":1,"peak_at":"s","mean_price":1,"intervals_above_5000":0,
            "cost_to_load":1,"max_demand_mw":1}"#;
        let h: Headline = serde_json::from_str(old).unwrap();
        assert!(h.tags.is_empty());
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
                    contingency: vec![Some("T-V-MNSP1".into())],
                    contingency_mw: vec![Some(478.0)],
                    reserve_mw: vec![Some(239.0)],
                    crr: vec![Some(0.5)],
                    ..RegionSeries::default()
                },
            )]),
        };
        let bytes = serde_json::to_vec(&s).unwrap();
        check_public_file("series.json", &bytes).unwrap();
        assert_eq!(serde_json::from_slice::<Series>(&bytes).unwrap(), s);
    }

    #[test]
    fn series_written_before_crr_still_reads() {
        let old = br#"{"schema":2,"intervals":["t"],"regions":{"SA1":{"price":[1.0],
            "demand":[null],"available_generation":[null],"net_interchange":[null]}}}"#;
        let s: Series = serde_json::from_slice(old).unwrap();
        assert!(s.regions["SA1"].crr.is_empty());
        assert!(!serde_json::to_string(&s).unwrap().contains("crr"));
        assert!(is_dc_link("T-V-MNSP1") && !is_dc_link("V-SA"));
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
