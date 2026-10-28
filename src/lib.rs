//! Pure parsers for AEMO NEMWeb data, shared across NEM Lens components.
//!
//! [NEMWeb](https://nemweb.com.au) publishes the National Electricity
//! Market's operational data as zipped CSV reports. This crate turns those
//! bytes into typed Rust values: dispatch prices and flows, generator offers,
//! market notices, and the NEM Lens event record built from them.
//!
//! Nothing here performs I/O or reads the system clock: callers fetch bytes
//! and pass them in. That keeps results deterministic and lets the crate
//! build for `wasm32-unknown-unknown`.
//!
//! # Modules
//!
//! - [`report`]: the generic C/I/D multi-table CSV format used by every
//!   report, collected in memory or streamed row by row.
//! - [`dispatch`]: typed DispatchIS snapshots (prices, region totals,
//!   constraints, interconnectors), pricing run only.
//! - [`bids`]: typed offers from next-day offer files, and the version in
//!   force for an interval.
//! - [`mnsp`]: offers of market network service providers (Basslink), which
//!   cap and price their links' flow.
//! - [`time`]: market time (AEST, interval end), trading days, regional clocks.
//! - [`files`]: NEMWeb file name patterns and unzipping (buffered or streamed).
//! - [`event`]: the NEM Lens event record schema, each file and field tagged
//!   public or private, and the gate that keeps the public library public.
//!
//! # Example
//!
//! A DispatchIS report, as published every five minutes, parsed into one
//! interval's prices. In practice the CSV comes from a zip fetched from
//! NEMWeb; [`files::unzip_csvs`] extracts it.
//!
//! ```
//! # fn main() -> anyhow::Result<()> {
//! let csv = "\
//! I,DISPATCH,PRICE,5,SETTLEMENTDATE,RUNNO,REGIONID,INTERVENTION,RRP
//! D,DISPATCH,PRICE,5,\"2026/10/12 14:35:00\",1,NSW1,0,23107.0
//! D,DISPATCH,PRICE,5,\"2026/10/12 14:35:00\",1,QLD1,0,180.5
//! ";
//! let tables = nem_lens_aemo::parse_report(csv.as_bytes())?;
//! let snap = nem_lens_aemo::dispatch_snapshot(&tables)?;
//! assert_eq!(snap.settlement.to_string(), "2026-10-12 14:35:00");
//! assert_eq!(snap.regions["NSW1"].rrp, 23107.0);
//! # Ok(())
//! # }
//! ```
//!
//! # Large files
//!
//! [`parse_report`] keeps every row in memory as strings. A next-day offer
//! file is about 250 MB of CSV, so stream it instead: [`files::visit_csvs`]
//! decompresses a member as it is read, and [`visit_report`],
//! [`parse_report_where`] and [`bids::read_offers`] handle one row at a time.
//!
//! # Data
//!
//! AEMO's data is © AEMO and used under
//! [AEMO's copyright permissions](https://www.aemo.com.au/privacy-and-legal-notices/copyright-permissions).
//! This crate contains no AEMO data, only code that reads it.

#![warn(missing_docs)]

pub mod bids;
pub mod dispatch;
pub mod event;
pub mod files;
pub mod mnsp;
pub mod report;
pub mod time;

pub use dispatch::{
    dispatch_snapshot, snapshot_from, ConstraintRow, DispatchSnapshot, InterconnectorRow,
    RegionInterval,
};
pub use report::{
    parse_report, parse_report_where, visit_report, Fields, Record, Table, TableRow, Tables,
};
