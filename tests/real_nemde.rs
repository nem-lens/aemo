//! Golden tests against extracts of real NEMDE case files (see
//! tests/data/README.md): the dispatch run for the interval ending 05:30 on
//! 10 Jul 2025, and 05:15 on 31 Jan 2020, the older layout without
//! bidirectional units or 1-second FCAS.
//!
//! Set NEMDE_CASES to a folder of whole `.loaded` case files to also read
//! each of them (`cargo test --test real_nemde -- --ignored`).

use nem_lens_aemo::nemde::{day_member, day_members, parse_case_name, read_case, CaseFile};

const JUL_2025: &[u8] = include_bytes!("data/NEMSPDOutputs_2025071001800_extract.zip");
const JAN_2020: &[u8] = include_bytes!("data/NEMSPDOutputs_2020013101500_extract.zip");

fn case(zip: &[u8]) -> CaseFile {
    let members = day_members(zip).unwrap();
    assert_eq!(members.len(), 1);
    let name = &members[0].0;
    assert!(parse_case_name(name).unwrap().is_dispatch_run());
    read_case(&day_member(zip, name, 64 << 20).unwrap()).unwrap()
}

#[test]
fn reads_the_case_and_its_penalty_prices() {
    let c = case(JUL_2025);
    assert_eq!(c.version, "2.1.0");
    assert_eq!(c.case.case_id, "20250710018");
    assert_eq!(c.case.case_type, "DS");
    assert!(!c.case.intervention);
    assert_eq!((c.case.voll, c.case.mpf), (20300.0, -1000.0));
    let p = c.case.prices();
    assert_eq!(p["TieBreakPrice"], 1e-6);
    assert_eq!(p["GenericConstraintPrice"], 609_000.0);
    assert_eq!(p["EnergyDeficitPrice"], 3_045_000.0);
    assert!(c.unread.is_empty(), "unread elements: {:?}", c.unread);
    assert_eq!(c.periods.len(), 1);
    assert_eq!(c.periods[0].period_id.to_string(), "2025-07-10 05:30:00");
}

#[test]
fn bidirectional_unit_prices_and_trades() {
    let c = case(JUL_2025);
    let adp = c.traders.iter().find(|t| t.trader_id == "ADPBA1").unwrap();
    assert_eq!(adp.trader_type, "BIDIRECTIONAL");
    assert_eq!(adp.initial_conditions["InitialMW"], -6.015);
    assert_eq!(adp.prices.len(), 12);
    // Referred to the regional node: the offer's -980 / (TLF 1.0037 x DLF 1.0312).
    let gen = &adp.prices[0];
    assert_eq!(
        (gen.trade_type.as_str(), gen.direction.as_deref()),
        ("BDOF", Some("GEN"))
    );
    assert_eq!(gen.price_bands[0], -946.85);
    assert_eq!(gen.price_bands[9], 9661.69);
    assert_eq!(adp.prices[1].direction.as_deref(), Some("LOAD"));
    assert_eq!(adp.prices[1].price_bands[9], 483.08);

    let period = c.periods[0]
        .traders
        .iter()
        .find(|t| t.trader_id == "ADPBA1")
        .unwrap();
    assert_eq!(period.region_id, "SA1");
    let load = period
        .trades
        .iter()
        .find(|t| t.trade_type == "BDOF" && t.direction.as_deref() == Some("LOAD"))
        .unwrap();
    assert_eq!(load.max_avail, 6.0);
    assert_eq!(load.band_avail[5], 6.0);
    assert_eq!(load.ramp_up_rate, Some(240.0));
}

#[test]
fn generator_trades_and_targets() {
    let c = case(JUL_2025);
    let er03 = c.periods[0]
        .traders
        .iter()
        .find(|t| t.trader_id == "ER03")
        .unwrap();
    let energy = er03.trades.iter().find(|t| t.trade_type == "ENOF").unwrap();
    assert_eq!(energy.direction, None);
    assert_eq!(energy.max_avail, 700.0);
    assert_eq!(
        energy.band_avail,
        [422.0, 0.0, 0.0, 25.0, 110.0, 90.0, 53.0, 0.0, 0.0, 50.0]
    );
    let sol = c
        .solution
        .traders
        .iter()
        .find(|t| t.trader_id == "ER03")
        .unwrap();
    assert_eq!(sol.energy_target, 580.81305);
    let bw02 = c
        .solution
        .traders
        .iter()
        .find(|t| t.trader_id == "BW02")
        .unwrap();
    assert_eq!(bw02.value("R6Target"), Some(5.0));
}

#[test]
fn interconnectors_loss_models_and_basslink() {
    let c = case(JUL_2025);
    assert_eq!(c.interconnectors.len(), 6);
    let nq = &c.interconnectors[0];
    assert_eq!(nq.interconnector_id, "N-Q-MNSP1");
    let lm = &nq.loss_models[0];
    assert_eq!((lm.loss_share, lm.loss_lower_limit), (0.66, 265.0));
    assert_eq!(lm.segments.len(), 66);
    assert_eq!(
        (lm.segments[0].limit, lm.segments[0].factor),
        (-257.0, -0.7074475)
    );

    let bass = c
        .interconnectors
        .iter()
        .find(|i| i.interconnector_id == "T-V-MNSP1")
        .unwrap();
    let links: Vec<_> = bass
        .mnsp_prices
        .iter()
        .map(|m| (m.link_id.as_str(), m.region_id.as_str(), m.price_bands[0]))
        .collect();
    assert_eq!(links, [("BLNKVIC", "TAS1", 0.01), ("BLNKTAS", "VIC1", 0.0)]);
    let period = c.periods[0]
        .interconnectors
        .iter()
        .find(|i| i.interconnector_id == "T-V-MNSP1")
        .unwrap();
    assert!(period.mnsp);
    assert_eq!((period.lower_limit, period.upper_limit), (478.0, 594.0));
    assert_eq!(period.mnsp_offers.len(), 2);
    assert_eq!(period.mnsp_offers[0].max_avail, 438.0);

    let sol = c
        .solution
        .interconnectors
        .iter()
        .find(|i| i.interconnector_id == "VIC1-NSW1")
        .unwrap();
    assert_eq!((sol.flow, sol.losses), (933.74157, 101.61819));
    assert_eq!(sol.inter_regional_loss_factor, Some(1.191904));
}

#[test]
fn constraints_with_every_kind_of_term() {
    let c = case(JUL_2025);
    assert_eq!(c.constraints.len(), 37);
    let fcas = c
        .constraints
        .iter()
        .find(|k| k.constraint_id == "D_I+BIP_ML2_L1")
        .unwrap();
    assert_eq!(fcas.constraint_type, "GE");
    assert_eq!((fcas.rhs, fcas.violation_price), (75.0, 182_700.0));
    assert!(!fcas.region_factors.is_empty());
    assert_eq!(fcas.rhs_terms.len(), 38);
    let n81 = c
        .constraints
        .iter()
        .find(|k| k.constraint_id == "N>>NIL_81")
        .unwrap();
    assert_eq!((n81.rhs, n81.violation_price), (7400.0, 7_308_000.0));
    assert!(!n81.trader_factors.is_empty() && !n81.interconnector_factors.is_empty());
    assert!(c.constraints.iter().all(|k| k.tracking.len() <= 1));
    assert_eq!(c.periods[0].constraints.len(), c.solution.constraints.len());
}

#[test]
fn prices_and_their_setters() {
    let c = case(JUL_2025);
    let nsw = c
        .solution
        .regions
        .iter()
        .find(|r| r.region_id == "NSW1")
        .unwrap();
    assert_eq!(nsw.energy_price, 77.30009);
    assert_eq!(nsw.net_export, -262.64);
    assert_eq!(nsw.value("R6Price"), Some(0.38));
    assert_eq!(c.price_setting.len(), 98);
    // A region's price is the sum of its setters' RRNBandPrice x Increase,
    // to rounding.
    for r in &c.solution.regions {
        let sum: f64 = c
            .price_setting
            .iter()
            .filter(|p| p.region_id == r.region_id && p.market == "Energy")
            .map(|p| p.rrn_band_price * p.increase)
            .sum();
        assert!(
            (sum - r.energy_price).abs() < 0.01,
            "{}: {sum}",
            r.region_id
        );
    }
}

#[test]
fn reads_the_2020_layout() {
    let c = case(JAN_2020);
    assert_eq!(c.case.voll, 14700.0);
    assert_eq!(c.case.prices()["GenericConstraintPrice"], 441_000.0);
    assert!(c.unread.is_empty(), "unread elements: {:?}", c.unread);
    let stan = c.periods[0]
        .traders
        .iter()
        .find(|t| t.trader_id == "STAN-1")
        .unwrap();
    let energy = &stan.trades[0];
    assert_eq!(energy.trade_type, "ENOF");
    assert_eq!(energy.direction, None);
    assert_eq!(energy.band_avail[0], 195.0);
    assert!(c.periods[0]
        .regions
        .iter()
        .all(|r| r.usage_factors.is_empty()));
    let nsw = c
        .solution
        .regions
        .iter()
        .find(|r| r.region_id == "NSW1")
        .unwrap();
    assert_eq!(nsw.energy_price, 63.87038);
    assert_eq!(nsw.value("R1Price"), None);
    assert!(nsw.value("ClearedDemand").is_some());
    let bass = c
        .interconnectors
        .iter()
        .find(|i| i.interconnector_id == "T-V-MNSP1")
        .unwrap();
    assert_eq!(bass.mnsp_prices[1].price_bands[0], -972.8);
}

#[test]
#[ignore]
fn reads_whole_case_files() {
    let Ok(dir) = std::env::var("NEMDE_CASES") else {
        return;
    };
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("loaded") {
            continue;
        }
        let t = std::time::Instant::now();
        let c = read_case(&std::fs::read(&path).unwrap()).unwrap();
        println!(
            "{}: {} traders, {} constraints, {} scada, unread {:?}, {:.2} s",
            path.display(),
            c.traders.len(),
            c.constraints.len(),
            c.scada.len(),
            c.unread,
            t.elapsed().as_secs_f64()
        );
        // Intervention cases solve twice, so units can have two solutions.
        assert!(c.solution.traders.len() >= c.traders.len());
        assert!(c.unread.is_empty(), "unread elements: {:?}", c.unread);
        if parse_case_name(&path.file_name().unwrap().to_string_lossy())
            .is_some_and(|n| !n.is_dispatch_run())
        {
            assert!(!c.solution.regions_original.is_empty());
            assert!(!c.price_setting_original.is_empty());
        }
    }
}
