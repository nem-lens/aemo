//! Golden tests against real AEMO offers (see tests/data/README.md): every row
//! for units ARWF1 (one version) and ADPBA1 (a battery, 294 versions per
//! direction) from the trading day 16 Oct 2026 file.

use std::collections::HashMap;

use chrono::{Duration, NaiveDate};
use nem_lens_aemo::bids::{
    period_end, read_offers, Direction, OfferFilter, Offers, PERIODS_PER_DAY,
};
use nem_lens_aemo::files::{visit_csvs, MAX_MEMBER_BYTES};
use nem_lens_aemo::time::parse_market;

const ZIP: &[u8] = include_bytes!("data/NEXT_DAY_OFFER_ENERGY_SPARSE_20261016_ADPBA1_ARWF1.zip");

fn offers(filter: &OfferFilter) -> Offers {
    let mut out = None;
    visit_csvs(ZIP, MAX_MEMBER_BYTES, |_, r| {
        out = Some(read_offers(r, filter)?);
        Ok(())
    })
    .unwrap();
    out.unwrap()
}

fn day() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 10, 16).unwrap()
}

#[test]
fn reads_every_row() {
    let o = offers(&OfferFilter::default());
    assert_eq!(o.day.len(), 589);
    assert_eq!(o.periods.len(), 5682);
    assert!(o
        .day
        .iter()
        .all(|d| d.trading_date == day() && d.bid_type == "ENERGY"));
}

#[test]
fn single_version_unit_matches_the_file() {
    let o = offers(&OfferFilter::default());
    let d = o.day.iter().find(|d| d.duid == "ARWF1").unwrap();
    assert_eq!(
        d.price_bands,
        [-898.0, -224.5, -197.56, -157.15, -127.52, -123.92, -112.25, 224.5, 2694.0, 20833.6]
    );
    assert_eq!(d.offer_time, parse_market("2026/09/22 11:21:32").unwrap());
    assert_eq!(d.participant_id, "ARARATWF");
    assert_eq!(d.rebid_explanation, "INITIAL OFFER SL");
    assert_eq!(d.direction, Direction::Gen);

    let idx = o.index();
    let end = period_end(day(), 127);
    let (_, p) = idx
        .effective(
            "ARWF1",
            "ENERGY",
            Direction::Gen,
            end,
            end - Duration::minutes(5),
        )
        .unwrap();
    assert_eq!(
        (p.period_from, p.period_to, p.max_avail),
        (1, 288, Some(235.0))
    );
    assert!(idx
        .effective("ARWF1", "ENERGY", Direction::Load, end, end)
        .is_none());
}

#[test]
fn every_version_tiles_the_day_exactly_once() {
    let o = offers(&OfferFilter::default());
    let mut cover: HashMap<_, Vec<u8>> = HashMap::new();
    for p in &o.periods {
        let c = cover
            .entry((p.duid.clone(), p.direction, p.offer_time))
            .or_insert_with(|| vec![0; PERIODS_PER_DAY as usize + 1]);
        for i in p.period_from..=p.period_to {
            c[i as usize] += 1;
        }
    }
    assert_eq!(cover.len(), o.day.len(), "one period set per day offer");
    assert!(cover.values().all(|c| c[1..].iter().all(|n| *n == 1)));
}

#[test]
fn effective_offer_is_the_latest_version_before_the_cutoff() {
    let o = offers(&OfferFilter::default());
    let idx = o.index();
    for dir in [Direction::Gen, Direction::Load] {
        let unit = ("ADPBA1".to_string(), "ENERGY".to_string(), dir, day());
        let versions = idx.versions(&unit);
        assert_eq!(versions.len(), 294);
        for period in (1..=PERIODS_PER_DAY).step_by(7) {
            let end = period_end(day(), period);
            let cutoff = end - Duration::minutes(5);
            let expected = o
                .day
                .iter()
                .filter(|d| d.duid == "ADPBA1" && d.direction == dir && d.offer_time <= cutoff)
                .map(|d| d.offer_time)
                .max();
            let got = idx.effective("ADPBA1", "ENERGY", dir, end, cutoff);
            assert_eq!(
                got.map(|(d, _)| d.offer_time),
                expected,
                "{dir:?} period {period}"
            );
            if let Some((d, p)) = got {
                assert!(p.covers(period));
                assert_eq!(p.offer_time, d.offer_time);
                assert_eq!(p.direction, dir);
            }
        }
    }
}

#[test]
fn filtering_while_streaming_keeps_versions_and_window() {
    let all = offers(&OfferFilter::default());
    let f = OfferFilter {
        duids: Some(["ADPBA1".to_string()].into()),
        periods: Some((121, 156)),
        ..Default::default()
    };
    let some = offers(&f);
    assert_eq!(some.day.len(), 588, "all ADPBA1 versions, both directions");
    assert!(some.periods.len() < all.periods.len());
    assert!(some
        .periods
        .iter()
        .all(|p| p.duid == "ADPBA1" && p.period_from <= 156 && p.period_to >= 121));
    // The window still answers every interval inside it.
    let (full, part) = (all.index(), some.index());
    for period in 121..=156 {
        let end = period_end(day(), period);
        let cut = end - Duration::minutes(5);
        let a = full.effective("ADPBA1", "ENERGY", Direction::Gen, end, cut);
        let b = part.effective("ADPBA1", "ENERGY", Direction::Gen, end, cut);
        assert_eq!(a, b, "period {period}");
    }
}
