//! Typed offers (bids) from AEMO's next-day offer files.
//!
//! `Next_Day_Offer_Energy_SPARSE` (and the FCAS equivalent) hold every offer
//! version submitted for a trading day, in two tables:
//!
//! - `BIDS,BIDDAYOFFER`: one row per version: price bands, daily limits and
//!   the rebid explanation.
//! - `BIDS,BIDOFFERPERIOD_SPARSE`: availability by band, ramp rates and so
//!   on, for runs of periods (`PERIODID` to `PERIODIDTO`).
//!
//! A version is identified by (DUID, BIDTYPE, DIRECTION, trading date, offer
//! time). Bidirectional units (batteries) bid `GEN` and `LOAD` separately. In
//! the 16 Oct 2026 file every period row joins exactly one day row on offer
//! time, and each version's period runs cover 1 to 288 exactly once.
//!
//! Period 1 is the interval ending 04:05 on the trading date; period 288 ends
//! at 04:00 the next morning.

use std::collections::{HashMap, HashSet};
use std::io::Read;

use anyhow::{Context, Result};
use chrono::{Duration, NaiveDate, NaiveDateTime, NaiveTime};

use crate::report::{visit_report, Fields};
use crate::time::trading_day;

pub const DAY_OFFER_TABLE: &str = "BIDS_BIDDAYOFFER";
pub const PERIOD_OFFER_TABLE: &str = "BIDS_BIDOFFERPERIOD_SPARSE";
pub const PERIODS_PER_DAY: u16 = 288;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Direction {
    Gen,
    Load,
}

impl Direction {
    /// `LOAD` is load; anything else, including a missing column in files
    /// from before bidirectional units, is generation.
    pub fn parse(s: Option<&str>) -> Self {
        match s.map(str::trim) {
            Some(d) if d.eq_ignore_ascii_case("LOAD") => Self::Load,
            _ => Self::Gen,
        }
    }
}

/// One offer version: `BIDS,BIDDAYOFFER`.
#[derive(Debug, Clone, PartialEq)]
pub struct DayOffer {
    pub duid: String,
    /// ENERGY, or an FCAS service such as RAISE6SEC.
    pub bid_type: String,
    pub direction: Direction,
    pub trading_date: NaiveDate,
    /// When the version was submitted (market time).
    pub offer_time: NaiveDateTime,
    pub version_no: Option<u32>,
    pub participant_id: String,
    /// $/MWh for bands 1 to 10.
    pub price_bands: [f64; 10],
    pub daily_energy_constraint: Option<f64>,
    pub minimum_load: Option<f64>,
    pub rebid_explanation: String,
    pub rebid_category: String,
    pub entry_type: String,
}

/// Quantities for a run of periods within one version: `BIDS,BIDOFFERPERIOD_SPARSE`.
#[derive(Debug, Clone, PartialEq)]
pub struct PeriodOffer {
    pub duid: String,
    pub bid_type: String,
    pub direction: Direction,
    pub trading_date: NaiveDate,
    /// Matches [`DayOffer::offer_time`] of the version this row belongs to.
    pub offer_time: NaiveDateTime,
    /// First and last period covered, inclusive (1 to 288).
    pub period_from: u16,
    pub period_to: u16,
    pub max_avail: Option<f64>,
    pub fixed_load: Option<f64>,
    pub ramp_up_rate: Option<f64>,
    pub ramp_down_rate: Option<f64>,
    pub enablement_min: Option<f64>,
    pub enablement_max: Option<f64>,
    pub low_breakpoint: Option<f64>,
    pub high_breakpoint: Option<f64>,
    /// MW available in bands 1 to 10.
    pub band_avail: [f64; 10],
    pub pasa_availability: Option<f64>,
    pub energy_limit: Option<f64>,
}

impl PeriodOffer {
    pub fn covers(&self, period: u16) -> bool {
        (self.period_from..=self.period_to).contains(&period)
    }
}

/// The interval end (market time) of `period` on `trading_date`.
pub fn period_end(trading_date: NaiveDate, period: u16) -> NaiveDateTime {
    trading_date.and_time(NaiveTime::from_hms_opt(4, 0, 0).expect("valid time"))
        + Duration::minutes(5 * i64::from(period))
}

/// The trading date and period of the interval ending at `interval_end`.
pub fn period_of(interval_end: NaiveDateTime) -> (NaiveDate, u16) {
    let day = trading_day(interval_end);
    let start = period_end(day, 0);
    (day, ((interval_end - start).num_minutes() / 5) as u16)
}

/// Which rows [`read_offers`] keeps. The default keeps everything.
#[derive(Debug, Clone, Default)]
pub struct OfferFilter {
    pub duids: Option<HashSet<String>>,
    pub bid_types: Option<HashSet<String>>,
    /// Keep period rows overlapping this inclusive range of periods.
    pub periods: Option<(u16, u16)>,
}

impl OfferFilter {
    fn unit_ok(&self, r: &impl Fields) -> bool {
        let has = |set: &Option<HashSet<String>>, col: &str| {
            set.as_ref()
                .is_none_or(|s| r.text(col).is_some_and(|v| s.contains(v.as_ref())))
        };
        has(&self.duids, "DUID") && has(&self.bid_types, "BIDTYPE")
    }
}

#[derive(Debug, Clone, Default)]
pub struct Offers {
    pub day: Vec<DayOffer>,
    pub periods: Vec<PeriodOffer>,
}

/// Stream a next-day offer CSV and keep the offers that pass `filter`.
/// Day offers are kept for every version of a kept unit, since choosing the
/// version in force needs them all.
pub fn read_offers<R: Read>(reader: R, filter: &OfferFilter) -> Result<Offers> {
    let mut out = Offers::default();
    visit_report(reader, |r| out.push(r, filter))?;
    Ok(out)
}

impl Offers {
    /// Keep `r` if it is a day or period offer row that passes `filter`;
    /// rows of other tables are ignored. [`read_offers`] calls this for
    /// every row of a file.
    pub fn push(&mut self, r: &impl Fields, filter: &OfferFilter) -> Result<()> {
        match r.key() {
            DAY_OFFER_TABLE if filter.unit_ok(r) => self.day.push(day_offer(r)?),
            PERIOD_OFFER_TABLE if filter.unit_ok(r) => {
                let p = period_offer(r)?;
                let overlaps = filter
                    .periods
                    .is_none_or(|(a, b)| p.period_from <= b && p.period_to >= a);
                if overlaps {
                    self.periods.push(p);
                }
            }
            _ => {}
        }
        Ok(())
    }
}

fn required<'a>(r: &'a impl Fields, col: &str) -> Result<std::borrow::Cow<'a, str>> {
    r.text(col)
        .with_context(|| format!("{} row is missing {col}", r.key()))
}

fn time(r: &impl Fields, col: &str) -> Result<NaiveDateTime> {
    let t = r.time(col);
    t.with_context(|| match r.text(col) {
        Some(s) => format!("{} {col} is not a market time: {s}", r.key()),
        None => format!("{} row is missing {col}", r.key()),
    })
}

fn bands(r: &impl Fields, prefix: &str) -> Result<[f64; 10]> {
    let mut out = [0.0; 10];
    for (i, v) in out.iter_mut().enumerate() {
        let col = format!("{prefix}{}", i + 1);
        *v = r
            .num(&col)
            .with_context(|| format!("{} row is missing {col}", r.key()))?;
    }
    Ok(out)
}

fn day_offer(r: &impl Fields) -> Result<DayOffer> {
    let text = |c: &str| r.text(c).unwrap_or_default().into_owned();
    Ok(DayOffer {
        duid: required(r, "DUID")?.into_owned(),
        bid_type: required(r, "BIDTYPE")?.into_owned(),
        direction: Direction::parse(r.text("DIRECTION").as_deref()),
        trading_date: time(r, "SETTLEMENTDATE")?.date(),
        offer_time: time(r, "OFFERDATE")?,
        version_no: r.text("VERSIONNO").and_then(|v| v.parse().ok()),
        participant_id: text("PARTICIPANTID"),
        price_bands: bands(r, "PRICEBAND")?,
        daily_energy_constraint: r.num("DAILYENERGYCONSTRAINT"),
        minimum_load: r.num("MINIMUMLOAD"),
        rebid_explanation: text("REBIDEXPLANATION"),
        rebid_category: text("REBID_CATEGORY"),
        entry_type: text("ENTRYTYPE"),
    })
}

fn period(r: &impl Fields, col: &str) -> Result<u16> {
    let p: u16 = required(r, col)?
        .parse()
        .with_context(|| format!("{} {col} is not a period", r.key()))?;
    anyhow::ensure!(
        (1..=PERIODS_PER_DAY).contains(&p),
        "{} {col} {p} is out of range",
        r.key()
    );
    Ok(p)
}

/// The first trading day of five-minute settlement. Before it, BIDPEROFFER
/// rows are per 30-minute trading interval (PERIODID 1 to 48, no PERIODIDTO).
fn five_minute_day() -> NaiveDate {
    NaiveDate::from_ymd_opt(2021, 10, 1).expect("valid date")
}

/// A time from the first of `cols` present: the sparse table and the older
/// BIDPEROFFER table name the same fields differently.
fn time_of(r: &impl Fields, cols: &[&str]) -> Result<NaiveDateTime> {
    let col = cols
        .iter()
        .find(|c| r.text(c).is_some())
        .copied()
        .unwrap_or(cols[0]);
    time(r, col)
}

/// The five-minute periods a row covers. A BIDPEROFFER row has PERIODID
/// alone: one period, or before five-minute settlement one 30-minute
/// trading interval, which is six five-minute periods.
fn periods(r: &impl Fields, day: NaiveDate) -> Result<(u16, u16)> {
    if r.text("PERIODIDTO").is_some() {
        return Ok((period(r, "PERIODID")?, period(r, "PERIODIDTO")?));
    }
    if day >= five_minute_day() {
        let p = period(r, "PERIODID")?;
        return Ok((p, p));
    }
    let p: u16 = required(r, "PERIODID")?
        .parse()
        .with_context(|| format!("{} PERIODID is not a period", r.key()))?;
    anyhow::ensure!(
        (1..=PERIODS_PER_DAY / 6).contains(&p),
        "{} PERIODID {p} is out of range for 30-minute periods",
        r.key()
    );
    Ok(((p - 1) * 6 + 1, p * 6))
}

fn period_offer(r: &impl Fields) -> Result<PeriodOffer> {
    let trading_date = time_of(r, &["TRADINGDATE", "SETTLEMENTDATE"])?.date();
    let (period_from, period_to) = periods(r, trading_date)?;
    anyhow::ensure!(
        period_from <= period_to,
        "{} PERIODID {period_from} is after PERIODIDTO {period_to}",
        r.key()
    );
    Ok(PeriodOffer {
        duid: required(r, "DUID")?.into_owned(),
        bid_type: required(r, "BIDTYPE")?.into_owned(),
        direction: Direction::parse(r.text("DIRECTION").as_deref()),
        trading_date,
        offer_time: time_of(r, &["OFFERDATETIME", "OFFERDATE"])?,
        period_from,
        period_to,
        max_avail: r.num("MAXAVAIL"),
        fixed_load: r.num("FIXEDLOAD"),
        ramp_up_rate: r.num("RAMPUPRATE"),
        ramp_down_rate: r.num("RAMPDOWNRATE"),
        enablement_min: r.num("ENABLEMENTMIN"),
        enablement_max: r.num("ENABLEMENTMAX"),
        low_breakpoint: r.num("LOWBREAKPOINT"),
        high_breakpoint: r.num("HIGHBREAKPOINT"),
        band_avail: bands(r, "BANDAVAIL")?,
        pasa_availability: r.num("PASAAVAILABILITY"),
        energy_limit: r.num("ENERGYLIMIT"),
    })
}

/// (DUID, BIDTYPE, DIRECTION, trading date): one unit's offers for one service and day.
pub type UnitKey = (String, String, Direction, NaiveDate);

/// Lookup from a unit to its offer versions, built by [`Offers::index`].
pub struct OfferIndex<'a> {
    /// Day offers per unit, sorted by offer time.
    versions: HashMap<UnitKey, Vec<&'a DayOffer>>,
    /// Period rows per (unit, offer time).
    periods: HashMap<(UnitKey, NaiveDateTime), Vec<&'a PeriodOffer>>,
}

fn key(duid: &str, bid_type: &str, direction: Direction, day: NaiveDate) -> UnitKey {
    (duid.to_string(), bid_type.to_string(), direction, day)
}

impl Offers {
    pub fn index(&self) -> OfferIndex<'_> {
        let mut versions: HashMap<UnitKey, Vec<&DayOffer>> = HashMap::new();
        for d in &self.day {
            versions
                .entry(key(&d.duid, &d.bid_type, d.direction, d.trading_date))
                .or_default()
                .push(d);
        }
        for v in versions.values_mut() {
            v.sort_by_key(|d| d.offer_time);
        }
        let mut periods: HashMap<(UnitKey, NaiveDateTime), Vec<&PeriodOffer>> = HashMap::new();
        for p in &self.periods {
            periods
                .entry((
                    key(&p.duid, &p.bid_type, p.direction, p.trading_date),
                    p.offer_time,
                ))
                .or_default()
                .push(p);
        }
        OfferIndex { versions, periods }
    }
}

impl<'a> OfferIndex<'a> {
    /// All versions for a unit and day, oldest first.
    pub fn versions(&self, unit: &UnitKey) -> &[&'a DayOffer] {
        self.versions.get(unit).map(Vec::as_slice).unwrap_or(&[])
    }

    /// The offer in force for the interval ending `interval_end`: the latest
    /// version submitted at or before `cutoff`, with its row for that period.
    ///
    /// Which rebids a dispatch run sees is AEMO's gate rule, so the caller
    /// chooses `cutoff` (for example the start of the interval). Returns
    /// `None` if no version qualifies or its period row was filtered out.
    pub fn effective(
        &self,
        duid: &str,
        bid_type: &str,
        direction: Direction,
        interval_end: NaiveDateTime,
        cutoff: NaiveDateTime,
    ) -> Option<(&'a DayOffer, &'a PeriodOffer)> {
        let (day, period) = period_of(interval_end);
        let unit = key(duid, bid_type, direction, day);
        let versions = self.versions.get(&unit)?;
        let n = versions.partition_point(|d| d.offer_time <= cutoff);
        let day_offer = *versions.get(n.checked_sub(1)?)?;
        let row = self
            .periods
            .get(&(unit, day_offer.offer_time))?
            .iter()
            .find(|p| p.covers(period))?;
        Some((day_offer, row))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::time::parse_market;

    const SAMPLE: &str = "\
I,BIDS,BIDDAYOFFER,1,DUID,BIDTYPE,SETTLEMENTDATE,OFFERDATE,VERSIONNO,PARTICIPANTID,PRICEBAND1,PRICEBAND2,PRICEBAND3,PRICEBAND4,PRICEBAND5,PRICEBAND6,PRICEBAND7,PRICEBAND8,PRICEBAND9,PRICEBAND10,DIRECTION
D,BIDS,BIDDAYOFFER,1,U1,ENERGY,2026/10/16 00:00:00,2026/10/15 12:00:00,1,P,-1000,0,1,2,3,4,5,6,7,20000,GEN
D,BIDS,BIDDAYOFFER,1,U1,ENERGY,2026/10/16 00:00:00,2026/10/16 14:00:00,2,P,-1000,0,1,2,3,4,5,6,7,15000,GEN
D,BIDS,BIDDAYOFFER,1,U1,ENERGY,2026/10/16 00:00:00,2026/10/15 12:00:00,1,P,-1000,0,1,2,3,4,5,6,7,300,LOAD
I,BIDS,BIDOFFERPERIOD_SPARSE,2,DUID,BIDTYPE,TRADINGDATE,OFFERDATETIME,PERIODID,PERIODIDTO,MAXAVAIL,BANDAVAIL1,BANDAVAIL2,BANDAVAIL3,BANDAVAIL4,BANDAVAIL5,BANDAVAIL6,BANDAVAIL7,BANDAVAIL8,BANDAVAIL9,BANDAVAIL10,DIRECTION
D,BIDS,BIDOFFERPERIOD_SPARSE,2,U1,ENERGY,2026/10/16 00:00:00,2026/10/15 12:00:00,1,120,100,100,0,0,0,0,0,0,0,0,0,GEN
D,BIDS,BIDOFFERPERIOD_SPARSE,2,U1,ENERGY,2026/10/16 00:00:00,2026/10/15 12:00:00,121,288,50,50,0,0,0,0,0,0,0,0,0,GEN
D,BIDS,BIDOFFERPERIOD_SPARSE,2,U1,ENERGY,2026/10/16 00:00:00,2026/10/16 14:00:00,1,288,80,0,0,0,0,0,0,0,0,0,80,GEN
D,BIDS,BIDOFFERPERIOD_SPARSE,2,U1,ENERGY,2026/10/16 00:00:00,2026/10/15 12:00:00,1,288,40,0,0,0,0,0,0,0,0,0,40,LOAD
";

    fn t(s: &str) -> NaiveDateTime {
        parse_market(s).unwrap()
    }

    #[test]
    fn periods_map_to_interval_ends() {
        let d = NaiveDate::from_ymd_opt(2026, 10, 16).unwrap();
        assert_eq!(period_end(d, 1), t("2026/10/16 04:05:00"));
        assert_eq!(period_end(d, 288), t("2026/10/17 04:00:00"));
        assert_eq!(period_of(t("2026/10/16 04:05:00")), (d, 1));
        assert_eq!(period_of(t("2026/10/17 04:00:00")), (d, 288));
        assert_eq!(period_of(t("2026/10/16 14:35:00")), (d, 127));
        for p in 1..=PERIODS_PER_DAY {
            assert_eq!(period_of(period_end(d, p)), (d, p));
        }
    }

    #[test]
    fn reads_typed_offers() {
        let o = read_offers(SAMPLE.as_bytes(), &OfferFilter::default()).unwrap();
        assert_eq!(o.day.len(), 3);
        assert_eq!(o.periods.len(), 4);
        assert_eq!(o.day[0].price_bands[9], 20000.0);
        assert_eq!(o.day[2].direction, Direction::Load);
        assert_eq!(o.periods[1].period_from, 121);
        assert_eq!(o.periods[2].band_avail[9], 80.0);
        assert_eq!(o.day[0].version_no, Some(1));
    }

    #[test]
    fn half_hour_bidperoffer_rows_cover_six_periods() {
        // Before five-minute settlement: BIDPEROFFER names its fields
        // differently and has one row per 30-minute trading interval.
        let old = "I,BIDS,BIDOFFERPERIOD_SPARSE,1,DUID,BIDTYPE,SETTLEMENTDATE,OFFERDATE,PERIODID,MAXAVAIL,BANDAVAIL1,BANDAVAIL2,BANDAVAIL3,BANDAVAIL4,BANDAVAIL5,BANDAVAIL6,BANDAVAIL7,BANDAVAIL8,BANDAVAIL9,BANDAVAIL10
D,BIDS,BIDOFFERPERIOD_SPARSE,1,U1,ENERGY,2020/01/31 00:00:00,2020/01/30 12:00:00.000,22,100,100,0,0,0,0,0,0,0,0,0
";
        let o = read_offers(old.as_bytes(), &OfferFilter::default()).unwrap();
        let p = &o.periods[0];
        assert_eq!((p.period_from, p.period_to), (127, 132));
        assert_eq!(p.offer_time, t("2020/01/30 12:00:00"));
        assert_eq!(
            p.trading_date,
            NaiveDate::from_ymd_opt(2020, 1, 31).unwrap()
        );
        // From five-minute settlement, a lone PERIODID is one period.
        let new = old.replace("2020/01/31", "2021/10/01");
        let o = read_offers(new.as_bytes(), &OfferFilter::default()).unwrap();
        assert_eq!((o.periods[0].period_from, o.periods[0].period_to), (22, 22));
    }

    #[test]
    fn effective_offer_follows_the_cutoff_and_period() {
        let o = read_offers(SAMPLE.as_bytes(), &OfferFilter::default()).unwrap();
        let idx = o.index();
        let at = |end: &str, cutoff: &str, dir| {
            idx.effective("U1", "ENERGY", dir, t(end), t(cutoff))
                .map(|(d, p)| (d.price_bands[9], p.max_avail))
        };
        // Before the rebid: version 1, period 127 is in the 121-288 run.
        assert_eq!(
            at("2026/10/16 14:35:00", "2026/10/16 13:59:59", Direction::Gen),
            Some((20000.0, Some(50.0)))
        );
        assert_eq!(
            at("2026/10/16 05:00:00", "2026/10/16 04:55:00", Direction::Gen),
            Some((20000.0, Some(100.0)))
        );
        // At and after the rebid's submission time: version 2.
        assert_eq!(
            at("2026/10/16 14:35:00", "2026/10/16 14:00:00", Direction::Gen),
            Some((15000.0, Some(80.0)))
        );
        assert_eq!(
            at(
                "2026/10/16 14:35:00",
                "2026/10/16 14:30:00",
                Direction::Load
            ),
            Some((300.0, Some(40.0)))
        );
        // Nothing submitted yet, or another trading day.
        assert_eq!(
            at("2026/10/16 14:35:00", "2026/10/15 11:00:00", Direction::Gen),
            None
        );
        assert_eq!(
            at("2026/10/17 14:35:00", "2026/10/17 14:30:00", Direction::Gen),
            None
        );
    }

    #[test]
    fn filter_keeps_units_services_and_overlapping_periods() {
        let mut f = OfferFilter {
            periods: Some((100, 130)),
            ..Default::default()
        };
        let o = read_offers(SAMPLE.as_bytes(), &f).unwrap();
        assert_eq!(o.day.len(), 3, "all versions kept");
        assert_eq!(o.periods.len(), 4, "each row overlaps 100-130");
        f.periods = Some((200, 210));
        let o = read_offers(SAMPLE.as_bytes(), &f).unwrap();
        assert_eq!(o.periods.len(), 3);
        f.duids = Some(HashSet::from(["OTHER".to_string()]));
        assert!(read_offers(SAMPLE.as_bytes(), &f).unwrap().day.is_empty());
        f.duids = None;
        f.bid_types = Some(HashSet::from(["RAISE6SEC".to_string()]));
        assert!(read_offers(SAMPLE.as_bytes(), &f)
            .unwrap()
            .periods
            .is_empty());
    }

    #[test]
    fn bad_rows_are_errors() {
        let bad_period = SAMPLE.replace(",121,288,50,", ",121,289,50,");
        assert!(read_offers(bad_period.as_bytes(), &OfferFilter::default()).is_err());
        let reversed = SAMPLE.replace(",121,288,50,", ",288,121,50,");
        assert!(read_offers(reversed.as_bytes(), &OfferFilter::default()).is_err());
        let no_band = SAMPLE.replace(",7,20000,GEN", ",7,,GEN");
        assert!(read_offers(no_band.as_bytes(), &OfferFilter::default()).is_err());
    }

    #[test]
    fn direction_defaults_to_generation() {
        assert_eq!(Direction::parse(None), Direction::Gen);
        assert_eq!(Direction::parse(Some(" load ")), Direction::Load);
        assert_eq!(Direction::parse(Some("GEN")), Direction::Gen);
    }
}
