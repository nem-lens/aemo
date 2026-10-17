//! Pure parsers for AEMO NEMWeb data, shared across NEM Lens components.
//!
//! Nothing here performs I/O or reads the system clock: callers fetch bytes
//! and pass them in. That keeps results deterministic and lets the crate
//! build for `wasm32-unknown-unknown`.
//!
//! - [`report`]: the generic C/I/D multi-table CSV format used by every report.
//! - [`dispatch`]: typed DispatchIS snapshots (prices, region totals,
//!   constraints, interconnectors), pricing run only.
//! - [`time`]: market time (AEST, interval end), trading days, regional clocks.
//! - [`files`]: NEMWeb file name patterns and unzipping.

pub mod dispatch;
pub mod files;
pub mod report;
pub mod time;

pub use dispatch::{
    dispatch_snapshot, ConstraintRow, DispatchSnapshot, InterconnectorRow, RegionInterval,
};
pub use report::{parse_report, Table, Tables};
