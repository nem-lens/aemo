//! Typed DispatchIS snapshots.

use std::collections::BTreeMap;

use anyhow::{Context, Result};
use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};

use crate::report::{Table, Tables};
use crate::time::parse_market;

/// One dispatch interval as published in a DispatchIS file (pricing run only).
#[derive(Debug, Clone, Default)]
pub struct DispatchSnapshot {
    /// Interval end, market time.
    pub settlement: NaiveDateTime,
    pub regions: BTreeMap<String, RegionInterval>,
    pub constraints: Vec<ConstraintRow>,
    pub interconnectors: Vec<InterconnectorRow>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RegionInterval {
    pub rrp: f64,
    pub demand: Option<f64>,
    pub available_gen: Option<f64>,
    pub net_interchange: Option<f64>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ConstraintRow {
    pub id: String,
    pub rhs: Option<f64>,
    pub lhs: Option<f64>,
    pub marginal: f64,
    pub violation: f64,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct InterconnectorRow {
    pub id: String,
    pub flow: f64,
    pub export_limit: Option<f64>,
    pub import_limit: Option<f64>,
    pub marginal: f64,
}

/// INTERVENTION = 0 is the pricing run, which sets the published price. A
/// missing column is treated as the pricing run.
pub fn is_pricing_run(t: &Table, row: &[String]) -> bool {
    t.col(row, "INTERVENTION")
        .map(|v| v.trim() == "0" || v.trim().is_empty())
        .unwrap_or(true)
}

/// Extract prices, region totals, constraints and interconnectors from a DispatchIS report.
pub fn dispatch_snapshot(tables: &Tables) -> Result<DispatchSnapshot> {
    let price = tables
        .get("DISPATCH_PRICE")
        .context("report has no DISPATCH,PRICE table")?;
    let mut snap = DispatchSnapshot::default();
    let mut settlement = None;

    for row in &price.rows {
        if !is_pricing_run(price, row) {
            continue;
        }
        let (Some(region), Some(rrp), Some(sd)) = (
            price.col(row, "REGIONID"),
            price.num(row, "RRP"),
            price.col(row, "SETTLEMENTDATE").and_then(parse_market),
        ) else {
            continue;
        };
        settlement = Some(settlement.map_or(sd, |s: NaiveDateTime| s.max(sd)));
        snap.regions.insert(
            region.trim().to_string(),
            RegionInterval {
                rrp,
                ..Default::default()
            },
        );
    }
    snap.settlement = settlement.context("no priced regions in report")?;

    if let Some(rs) = tables.get("DISPATCH_REGIONSUM") {
        for row in &rs.rows {
            if !is_pricing_run(rs, row) {
                continue;
            }
            if let Some(r) = rs
                .col(row, "REGIONID")
                .and_then(|id| snap.regions.get_mut(id.trim()))
            {
                r.demand = rs.num(row, "TOTALDEMAND");
                r.available_gen = rs.num(row, "AVAILABLEGENERATION");
                r.net_interchange = rs.num(row, "NETINTERCHANGE");
            }
        }
    }

    if let Some(c) = tables.get("DISPATCH_CONSTRAINT") {
        for row in &c.rows {
            if !is_pricing_run(c, row) {
                continue;
            }
            let Some(id) = c.col(row, "CONSTRAINTID") else {
                continue;
            };
            snap.constraints.push(ConstraintRow {
                id: id.trim().to_string(),
                rhs: c.num(row, "RHS"),
                lhs: c.num(row, "LHS"),
                marginal: c.num(row, "MARGINALVALUE").unwrap_or(0.0),
                violation: c.num(row, "VIOLATIONDEGREE").unwrap_or(0.0),
            });
        }
    }

    if let Some(ic) = tables.get("DISPATCH_INTERCONNECTORRES") {
        for row in &ic.rows {
            if !is_pricing_run(ic, row) {
                continue;
            }
            let Some(id) = ic.col(row, "INTERCONNECTORID") else {
                continue;
            };
            snap.interconnectors.push(InterconnectorRow {
                id: id.trim().to_string(),
                flow: ic.num(row, "MWFLOW").unwrap_or(0.0),
                export_limit: ic.num(row, "EXPORTLIMIT"),
                import_limit: ic.num(row, "IMPORTLIMIT"),
                marginal: ic.num(row, "MARGINALVALUE").unwrap_or(0.0),
            });
        }
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
