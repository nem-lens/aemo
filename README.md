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

```rust
let members = nem_lens_aemo::files::unzip_csvs(&zip_bytes)?;
let tables = nem_lens_aemo::parse_report(&members[0].1)?;
let snap = nem_lens_aemo::dispatch_snapshot(&tables)?;
```

Other reports (bids, unit solutions, next-day files) parse with `parse_report`
and are read by table key, for example `tables["BIDS_BIDPEROFFER_D"]`.

## Development

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo build --target wasm32-unknown-unknown
```

`tests/data` holds one real DispatchIS file used as a golden test. Source data
© AEMO, used under AEMO's copyright permissions.
