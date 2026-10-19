//! Pure parsers for AEMO NEMWeb data, shared across NEM Lens components.
//!
//! Nothing here performs I/O or reads the system clock: callers fetch bytes
//! and pass them in. That keeps results deterministic and lets the crate
//! build for `wasm32-unknown-unknown`.
//!
//! - [`report`]: the generic C/I/D multi-table CSV format used by every
//!   report, collected in memory or streamed row by row.
//! - [`dispatch`]: typed DispatchIS snapshots (prices, region totals,
//!   constraints, interconnectors), pricing run only.
//! - [`bids`]: typed offers from next-day offer files, and the version in
//!   force for an interval.
//! - [`time`]: market time (AEST, interval end), trading days, regional clocks.
//! - [`files`]: NEMWeb file name patterns and unzipping (buffered or streamed).
//! - [`event`]: the NEM Lens event record schema, each file and field tagged
//!   public or private, and the gate that keeps the public library public.

pub mod bids;
pub mod dispatch;
pub mod event;
pub mod files;
pub mod report;
pub mod time;

pub use dispatch::{
    dispatch_snapshot, snapshot_from, ConstraintRow, DispatchSnapshot, InterconnectorRow,
    RegionInterval,
};
pub use report::{
    parse_report, parse_report_where, visit_report, Fields, Record, Table, TableRow, Tables,
};
