# nem-lens-aemo

Pure Rust parsers for AEMO NEMWeb data, shared by the NEM Lens components
(the `monitor` event detector and the replay `engine`).

The crate performs no I/O and never reads the system clock. Callers fetch the
bytes and pass them in, so results are deterministic and the crate builds for
`wasm32-unknown-unknown`.

| Module | Contents |
| --- | --- |
| `report` | The generic C/I/D multi-table CSV format; columns looked up by name |
| `dispatch` | Typed DispatchIS snapshots: prices, region totals, constraints, interconnectors (pricing run only) |
| `time` | Market time (AEST, UTC+10, interval end), the 04:00 trading day, regional local clocks |
| `files` | NEMWeb file name patterns (DispatchIS, Next_Day_*, Bidmove_*) and unzipping with a per-member size limit |
| `bids` | Typed offers from next-day offer files, and the version in force for an interval |
| `mnsp` | Offers of market network service providers (Basslink), which cap and price their links' flow |
| `nemde` | NEMDE case files from AEMO's NEMDE archive: the dispatch engine's inputs (penalty prices, referred band prices, availability, loss models, MNSP offers, generic constraints) and its solution and price setters, per interval, 2020 onwards |
| `event` | The NEM Lens event record schema, each file and field public or private, and the gate for the public library |

API documentation, with examples: https://nem-lens.github.io/aemo/nem_lens_aemo/
(built by rustdoc from `main`).

```rust
let members = nem_lens_aemo::files::unzip_csvs(&zip_bytes)?;
let tables = nem_lens_aemo::parse_report(&members[0].1)?;
let snap = nem_lens_aemo::dispatch_snapshot(&tables)?;
```

Other reports (unit solutions, next-day dispatch) parse with `parse_report`
and are read by table key.

### Large files: stream them

`parse_report` keeps every row in memory as strings. For large files, stream:
`files::visit_csvs` decompresses a member as it is read and
`visit_report` / `parse_report_where` handle one row at a time.

```rust
use nem_lens_aemo::bids::{read_offers, Direction, OfferFilter};

let filter = OfferFilter {
    duids: Some(["ADPBA1".to_string()].into()),
    periods: Some((121, 156)), // 14:05 to 17:00 market time
    ..Default::default()
};
let mut offers = None;
nem_lens_aemo::files::visit_csvs(&zip_bytes, nem_lens_aemo::files::MAX_MEMBER_BYTES, |_, r| {
    offers = Some(read_offers(r, &filter)?);
    Ok(())
})?;
let offers = offers.unwrap();
let index = offers.index();
// The version submitted at or before `cutoff`, with its row for the period.
let (day, period) = index.effective("ADPBA1", "ENERGY", Direction::Gen, interval_end, cutoff).unwrap();
```

On the trading day 16 Oct 2026 file (250 MB of CSV, 1.6 million rows):

| Approach | Time | Peak memory |
| --- | --- | --- |
| `parse_report` (all rows as strings) | 2.5 s | about 2 GB |
| `read_offers`, no filter (typed) | 4.5 s | 692 MB |
| `read_offers`, one unit and a 3-hour window | 0.8 s | 16 MB |

### Offers (`bids`)

`Next_Day_Offer_Energy_SPARSE` holds every offer version for a trading day in
`BIDS_BIDDAYOFFER` (price bands, one row per version) and
`BIDS_BIDOFFERPERIOD_SPARSE` (band availability and ramp rates for runs of
periods, `PERIODID` to `PERIODIDTO`). In the 16 Oct 2026 file, every period
row joins exactly one day row on (DUID, DIRECTION, offer time), and each
version's runs cover periods 1 to 288 exactly once. Batteries bid `GEN` and
`LOAD` separately. Period 1 is the interval ending 04:05.

`OfferIndex::effective` returns the latest version submitted at or before a
cutoff the caller chooses; which rebids a dispatch run sees is AEMO's gate
rule, so it is left to the engine.

## Development

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo build --target wasm32-unknown-unknown
```

`tests/data` holds one real DispatchIS file used as a golden test. Source data
© AEMO, used under AEMO's copyright permissions.
