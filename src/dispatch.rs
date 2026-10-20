//! Typed DispatchIS snapshots.
//!
//! AEMO publishes a DispatchIS report for every five-minute dispatch
//! interval. It holds the regional prices (DISPATCH,PRICE), region totals
//! (DISPATCH,REGIONSUM), generic constraints (DISPATCH,CONSTRAINT) and
//! interconnector flows and limits (DISPATCH,INTERCONNECTORRES).
//!
//! When AEMO intervenes in the market (a direction or RERT), each table
//! carries two runs: the pricing run (INTERVENTION = 0), which sets the
//! published price, and the intervention run, which sets physical dispatch.
//! Only the pricing run is read here.

use std::collections::BTreeMap;

use anyhow::{Context, Result};
use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};

use crate::report::{Fields, Table, Tables};

/// One dispatch interval as published in a DispatchIS file (pricing run only).
#[derive(Debug, Clone, Default)]
pub struct DispatchSnapshot {
    /// Interval end, market time.
    pub settlement: NaiveDateTime,
    /// Each priced region by REGIONID, e.g. "NSW1".
    pub regions: BTreeMap<String, RegionInterval>,
    /// Generic constraints invoked in the interval.
    pub constraints: Vec<ConstraintRow>,
    /// Every interconnector's flow and limits.
    pub interconnectors: Vec<InterconnectorRow>,
}

/// One region's figures for an interval (DISPATCH,PRICE and REGIONSUM).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RegionInterval {
    /// Regional reference price (RRP), $/MWh.
    pub rrp: f64,
    /// TOTALDEMAND, MW: scheduled and semi-scheduled demand.
    pub demand: Option<f64>,
    /// AVAILABLEGENERATION, MW.
    pub available_gen: Option<f64>,
    /// NETINTERCHANGE, MW, positive for net export.
    pub net_interchange: Option<f64>,
}

/// One generic constraint in an interval (DISPATCH,CONSTRAINT).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ConstraintRow {
    /// CONSTRAINTID, e.g. "N::N_CNCW_2".
    pub id: String,
    /// The constraint's right-hand side, in its units (MW for most).
    pub rhs: Option<f64>,
    /// The left-hand side as dispatched.
    pub lhs: Option<f64>,
    /// MARGINALVALUE, $/MWh: the cost of the constraint to the market. Zero
    /// when it does not bind.
    pub marginal: f64,
    /// VIOLATIONDEGREE: how far dispatch fell short of meeting the
    /// constraint. Zero when it was met.
    pub violation: f64,
}

/// One interconnector in an interval (DISPATCH,INTERCONNECTORRES).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct InterconnectorRow {
    /// INTERCONNECTORID, e.g. "VIC1-NSW1".
    pub id: String,
    /// MWFLOW, MW, positive in the interconnector's nominal direction (see
    /// [`crate::event::interconnector_ends`]).
    pub flow: f64,
    /// EXPORTLIMIT, MW: the upper bound on `flow`.
    pub export_limit: Option<f64>,
    /// IMPORTLIMIT, MW: the lower bound on `flow`, usually negative.
    pub import_limit: Option<f64>,
    /// MARGINALVALUE, $/MWh: non-zero when a limit binds.
    pub marginal: f64,
}

/// INTERVENTION = 0 is the pricing run, which sets the published price. A
/// missing column is treated as the pricing run.
pub fn is_pricing_run(t: &Table, row: &[String]) -> bool {
    t.col(row, "INTERVENTION")
        .map(|v| v.trim() == "0" || v.trim().is_empty())
        .unwrap_or(true)
}

/// [`is_pricing_run`] for any row.
pub fn is_pricing_row(r: &impl Fields) -> bool {
    r.text("INTERVENTION").is_none_or(|v| v == "0")
}

/// Extract prices, region totals, constraints and interconnectors from a
/// DispatchIS report, as parsed by [`crate::parse_report`].
///
/// # Errors
///
/// If the report has no DISPATCH,PRICE table, or no pricing-run price with a
/// region and settlement date.
pub fn dispatch_snapshot(tables: &Tables) -> Result<DispatchSnapshot> {
    let price = tables
        .get("DISPATCH_PRICE")
        .context("report has no DISPATCH,PRICE table")?;
    let rows = |key: &'static str| {
        tables
            .get(key)
            .into_iter()
            .flat_map(move |t| t.records(key))
    };
    snapshot_from(
        price.records("DISPATCH_PRICE"),
        rows("DISPATCH_REGIONSUM"),
        rows("DISPATCH_CONSTRAINT"),
        rows("DISPATCH_INTERCONNECTORRES"),
    )
}

/// [`dispatch_snapshot`] from one interval's rows of DISPATCH,PRICE,
/// REGIONSUM, CONSTRAINT and INTERCONNECTORRES, from any source of
/// [`Fields`]. Intervention-run rows are skipped, and the settlement time is
/// the latest among the prices.
///
/// # Errors
///
/// If no pricing-run price row has a region, price and settlement date.
pub fn snapshot_from<F: Fields>(
    price: impl IntoIterator<Item = F>,
    regionsum: impl IntoIterator<Item = F>,
    constraint: impl IntoIterator<Item = F>,
    interconnectorres: impl IntoIterator<Item = F>,
) -> Result<DispatchSnapshot> {
    let mut snap = DispatchSnapshot::default();
    let mut settlement = None;

    for r in price {
        if !is_pricing_row(&r) {
            continue;
        }
        let (Some(region), Some(rrp), Some(sd)) =
            (r.text("REGIONID"), r.num("RRP"), r.time("SETTLEMENTDATE"))
        else {
            continue;
        };
        settlement = Some(settlement.map_or(sd, |s: NaiveDateTime| s.max(sd)));
        snap.regions.insert(
            region.into_owned(),
            RegionInterval {
                rrp,
                ..Default::default()
            },
        );
    }
    snap.settlement = settlement.context("no priced regions in report")?;

    for r in regionsum {
        if !is_pricing_row(&r) {
            continue;
        }
        if let Some(x) = r
            .text("REGIONID")
            .and_then(|id| snap.regions.get_mut(id.as_ref()))
        {
            x.demand = r.num("TOTALDEMAND");
            x.available_gen = r.num("AVAILABLEGENERATION");
            x.net_interchange = r.num("NETINTERCHANGE");
        }
    }

    for r in constraint {
        if !is_pricing_row(&r) {
            continue;
        }
        let Some(id) = r.text("CONSTRAINTID") else {
            continue;
        };
        snap.constraints.push(ConstraintRow {
            id: id.into_owned(),
            rhs: r.num("RHS"),
            lhs: r.num("LHS"),
            marginal: r.num("MARGINALVALUE").unwrap_or(0.0),
            violation: r.num("VIOLATIONDEGREE").unwrap_or(0.0),
        });
    }

    for r in interconnectorres {
        if !is_pricing_row(&r) {
            continue;
        }
        let Some(id) = r.text("INTERCONNECTORID") else {
            continue;
        };
        snap.interconnectors.push(InterconnectorRow {
            id: id.into_owned(),
            flow: r.num("MWFLOW").unwrap_or(0.0),
            export_limit: r.num("EXPORTLIMIT"),
            import_limit: r.num("IMPORTLIMIT"),
            marginal: r.num("MARGINALVALUE").unwrap_or(0.0),
        });
    }
    Ok(snap)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::parse_report;

    const SAMPLE: &str = r#"C,NEMP.WORLD,DISPATCHIS,AEMO,PUBLIC,2026/10/12,14:30:13,0000000542000000,DISPATCHIS,0000000542000000
I,DISPATCH,PRICE,5,SETTLEMENTDATE,RUNNO,REGIONID,DISPATCHINTERVAL,INTERVENTION,RRP,EEP,ROP,APCFLAG
D,DISPATCH,PRICE,5,"2026/10/12 14:35:00",1,NSW1,20261012171,0,23107.0,0,23107.0,0
D,DISPATCH,PRICE,5,"2026/10/12 14:35:00",1,QLD1,20261012171,0,180.5,0,180.5,0
I,DISPATCH,REGIONSUM,9,SETTLEMENTDATE,RUNNO,REGIONID,DISPATCHINTERVAL,INTERVENTION,TOTALDEMAND,AVAILABLEGENERATION,NETINTERCHANGE
D,DISPATCH,REGIONSUM,9,"2026/10/12 14:35:00",1,NSW1,20261012171,0,12650.2,12980.0,-1400.0
I,DISPATCH,CONSTRAINT,5,SETTLEMENTDATE,RUNNO,CONSTRAINTID,DISPATCHINTERVAL,INTERVENTION,RHS,MARGINALVALUE,VIOLATIONDEGREE,LHS
D,DISPATCH,CONSTRAINT,5,"2026/10/12 14:35:00",1,N>>N-EXAMPLE_1,20261012171,0,850,-3200.5,0,850
D,DISPATCH,CONSTRAINT,5,"2026/10/12 14:35:00",1,N>>N-EXAMPLE_1,20261012171,1,850,-10,0,850
C,"END OF REPORT",9
"#;

    #[test]
    fn parses_dispatch_report() {
        let tables = parse_report(SAMPLE.as_bytes()).unwrap();
        let snap = dispatch_snapshot(&tables).unwrap();
        assert_eq!(snap.settlement.to_string(), "2026-10-12 14:35:00");
        assert_eq!(snap.regions["NSW1"].rrp, 23107.0);
        assert_eq!(snap.regions["NSW1"].demand, Some(12650.2));
        assert_eq!(snap.regions["QLD1"].demand, None);
        // The intervention run row (INTERVENTION=1) is skipped.
        assert_eq!(snap.constraints.len(), 1);
        assert_eq!(snap.constraints[0].marginal, -3200.5);
        assert_eq!(snap.constraints[0].rhs, Some(850.0));
    }

    #[test]
    fn intervention_only_prices_are_an_error() {
        let csv = "I,DISPATCH,PRICE,5,SETTLEMENTDATE,REGIONID,INTERVENTION,RRP\n\
                   D,DISPATCH,PRICE,5,2026/10/12 14:35:00,NSW1,1,300\n";
        let tables = parse_report(csv.as_bytes()).unwrap();
        assert!(dispatch_snapshot(&tables).is_err());
    }

    #[test]
    fn missing_price_table_is_an_error() {
        let csv = "I,DISPATCH,REGIONSUM,9,REGIONID\nD,DISPATCH,REGIONSUM,9,NSW1\n";
        let err = dispatch_snapshot(&parse_report(csv.as_bytes()).unwrap()).unwrap_err();
        assert!(err.to_string().contains("DISPATCH,PRICE"));
    }

    #[test]
    fn missing_intervention_column_counts_as_pricing_run() {
        let csv = "I,DISPATCH,PRICE,5,SETTLEMENTDATE,REGIONID,RRP\n\
                   D,DISPATCH,PRICE,5,2026/10/12 14:35:00,SA1,-40\n";
        let snap = dispatch_snapshot(&parse_report(csv.as_bytes()).unwrap()).unwrap();
        assert_eq!(snap.regions["SA1"].rrp, -40.0);
    }
}
