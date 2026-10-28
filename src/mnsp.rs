//! Typed offers of market network service providers (MNSPs).
//!
//! An MNSP link is an interconnector that offers its transfer into the
//! market, as a generator offers energy. Basslink (`T-V-MNSP1`) is one: two
//! links, `BLNKTAS` (Tasmania to Victoria) and `BLNKVIC` (back), each
//! offering MW in ten price bands. NEMDE dispatches a link only as far as
//! its offer allows, so its MAXAVAIL caps flow even where the
//! interconnector's limit (DISPATCHINTERCONNECTORRES EXPORTLIMIT) is
//! higher: on 26 Jun 2025 Basslink flowed 556, 535, 521 … 497 MW, its
//! MAXAVAIL, against a limit of 594.
//!
//! MMSDM publishes three tables:
//!
//! - `MNSP_INTERCONNECTOR`: which links make up an interconnector, and
//!   their direction.
//! - `MNSP_DAYOFFER`: one row per offer version: the ten price bands.
//! - `MNSP_BIDOFFERPERIOD` (five-minute periods), or before five-minute
//!   settlement `MNSP_PEROFFER` (30-minute): MAXAVAIL, ramp rate and MW per
//!   band, by period.
//!
//! A version is identified by (link, trading date, offer time), as unit
//! offers are (see [`crate::bids`]). As there, a standing offer keeps its
//! period rows under each trading day it applies to, while its prices are
//! under the day it was submitted for: [`MnspOffers::effective`] finds them
//! by link and offer time.
//!
//! # Example
//!
//! Basslink's TAS-to-VIC offer in force for the interval ending 18:40 on
//! 26 Jun 2025 (AEMO's figures; one band and period shown).
//!
//! ```
//! use nem_lens_aemo::mnsp::MnspOffers;
//! use nem_lens_aemo::report::visit_report;
//! use nem_lens_aemo::time::parse_market;
//!
//! # fn main() -> anyhow::Result<()> {
//! let csv = "\
//! I,MNSP,DAYOFFER,1,SETTLEMENTDATE,OFFERDATE,VERSIONNO,LINKID,PRICEBAND1,PRICEBAND2,PRICEBAND3,PRICEBAND4,PRICEBAND5,PRICEBAND6,PRICEBAND7,PRICEBAND8,PRICEBAND9,PRICEBAND10
//! D,MNSP,DAYOFFER,1,2025/06/26 00:00:00,2025/06/26 18:33:51,1,BLNKTAS,-962,-23.99,0.01,15.01,30.01,60.01,100.01,300.01,7500.01,17500
//! I,BIDS,MNSP_BIDOFFERPERIOD,1,LINKID,TRADINGDATE,OFFERDATETIME,PERIODID,MAXAVAIL,RAMPUPRATE,BANDAVAIL1,BANDAVAIL2,BANDAVAIL3,BANDAVAIL4,BANDAVAIL5,BANDAVAIL6,BANDAVAIL7,BANDAVAIL8,BANDAVAIL9,BANDAVAIL10
//! D,BIDS,MNSP_BIDOFFERPERIOD,1,BLNKTAS,2025/06/26 00:00:00,2025/06/26 18:33:51,176,556,200,0,0,594,0,0,0,0,0,0,0
//! ";
//! let mut offers = MnspOffers::default();
//! visit_report(csv.as_bytes(), |r| offers.push(r))?;
//! let t = |s| parse_market(s).unwrap();
//! let (day, period) = offers
//!     .effective("BLNKTAS", t("2025/06/26 18:40:00"), t("2025/06/26 18:35:00"))
//!     .unwrap();
//! assert_eq!(period.max_avail, 556.0);
//! assert_eq!(day.unwrap().price_bands[2], 0.01);
//! // Its bands, cut off at MAXAVAIL: 556 MW at $0.01.
//! assert_eq!(offers.bands("BLNKTAS", t("2025/06/26 18:40:00"), t("2025/06/26 18:35:00")),
//!            Some(vec![(556.0, 0.01)]));
//! # Ok(())
//! # }
//! ```

use std::collections::HashMap;

use anyhow::Result;
use chrono::{NaiveDate, NaiveDateTime};

use crate::bids::{bands, period, period_of, periods, required, time, time_of};
use crate::report::Fields;

/// One link of an MNSP interconnector (`MNSP_INTERCONNECTOR`).
#[derive(Debug, Clone, PartialEq)]
pub struct MnspLink {
    /// LINKID, e.g. "BLNKTAS".
    pub link_id: String,
    /// The interconnector it belongs to, e.g. "T-V-MNSP1".
    pub interconnector_id: String,
    /// The region it takes energy from.
    pub from_region: String,
    /// The region it delivers to.
    pub to_region: String,
    /// MAXCAPACITY, MW.
    pub max_capacity: Option<f64>,
    /// FROM_REGION_TLF and TO_REGION_TLF: the loss factors of the link's
    /// connection points, which refer its offer prices to the regional
    /// reference nodes (NER 3.8.6A(g)-(h)). Absent before 2012.
    pub from_region_tlf: Option<f64>,
    /// See `from_region_tlf`.
    pub to_region_tlf: Option<f64>,
    /// When this version takes effect.
    pub effective: NaiveDateTime,
    /// VERSIONNO.
    pub version_no: u32,
}

/// One MNSP offer version (`MNSP_DAYOFFER`).
#[derive(Debug, Clone, PartialEq)]
pub struct MnspDayOffer {
    /// LINKID.
    pub link_id: String,
    /// The trading day the offer was submitted for.
    pub trading_date: NaiveDate,
    /// When the version was submitted (market time).
    pub offer_time: NaiveDateTime,
    /// VERSIONNO, where published.
    pub version_no: Option<u32>,
    /// $/MWh for bands 1 to 10.
    pub price_bands: [f64; 10],
}

/// An MNSP version's quantities for a run of five-minute periods
/// (`MNSP_BIDOFFERPERIOD`, or `MNSP_PEROFFER` before five-minute
/// settlement: one row per 30-minute interval, six periods).
#[derive(Debug, Clone, PartialEq)]
pub struct MnspPeriodOffer {
    /// LINKID.
    pub link_id: String,
    /// The trading day the row applies to.
    pub trading_date: NaiveDate,
    /// Matches [`MnspDayOffer::offer_time`] of its version.
    pub offer_time: NaiveDateTime,
    /// VERSIONNO, where published (MNSP_PEROFFER).
    pub version_no: Option<u32>,
    /// First period covered, inclusive (1 to 288).
    pub period_from: u16,
    /// Last period covered, inclusive.
    pub period_to: u16,
    /// MAXAVAIL, MW: the most the link will transfer.
    pub max_avail: f64,
    /// RAMPUPRATE, MW per minute, where given.
    pub ramp_up_rate: Option<f64>,
    /// MW in bands 1 to 10.
    pub band_avail: [f64; 10],
}

impl MnspPeriodOffer {
    /// Whether this row covers `period`.
    pub fn covers(&self, period: u16) -> bool {
        (self.period_from..=self.period_to).contains(&period)
    }
}

/// MNSP links and offers read from MMSDM tables.
#[derive(Debug, Clone, Default)]
pub struct MnspOffers {
    /// Links, every version, in file order.
    pub links: Vec<MnspLink>,
    /// Day offers, one per version, in file order.
    pub day: Vec<MnspDayOffer>,
    /// Period offers, in file order.
    pub periods: Vec<MnspPeriodOffer>,
}

/// Which MNSP table a row belongs to, from its table key (the report part
/// varies across archive years).
pub fn table_of(key: &str) -> Option<MnspTable> {
    if key.ends_with("MNSP_INTERCONNECTOR") {
        Some(MnspTable::Link)
    } else if key.ends_with("DAYOFFER") && key.contains("MNSP") {
        Some(MnspTable::DayOffer)
    } else if key.ends_with("MNSP_BIDOFFERPERIOD") || key.ends_with("MNSP_PEROFFER") {
        Some(MnspTable::PeriodOffer)
    } else {
        None
    }
}

/// The three MNSP tables.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MnspTable {
    /// `MNSP_INTERCONNECTOR`.
    Link,
    /// `MNSP_DAYOFFER`.
    DayOffer,
    /// `MNSP_BIDOFFERPERIOD` or `MNSP_PEROFFER`.
    PeriodOffer,
}

impl MnspOffers {
    /// Keep `r` if it is a row of one of the MNSP tables; other rows are
    /// ignored.
    ///
    /// # Errors
    ///
    /// If a row lacks a required field (link, times, periods, the bands) or
    /// has a period out of range.
    pub fn push(&mut self, r: &impl Fields) -> Result<()> {
        match table_of(r.key()) {
            Some(MnspTable::Link) => self.links.push(MnspLink {
                link_id: required(r, "LINKID")?.into_owned(),
                interconnector_id: required(r, "INTERCONNECTORID")?.into_owned(),
                from_region: required(r, "FROMREGION")?.into_owned(),
                to_region: required(r, "TOREGION")?.into_owned(),
                max_capacity: r.num("MAXCAPACITY"),
                from_region_tlf: r.num("FROM_REGION_TLF"),
                to_region_tlf: r.num("TO_REGION_TLF"),
                effective: time(r, "EFFECTIVEDATE")?,
                version_no: r.num("VERSIONNO").map_or(0, |v| v as u32),
            }),
            Some(MnspTable::DayOffer) => self.day.push(MnspDayOffer {
                link_id: required(r, "LINKID")?.into_owned(),
                trading_date: time(r, "SETTLEMENTDATE")?.date(),
                offer_time: time(r, "OFFERDATE")?,
                version_no: r.num("VERSIONNO").map(|v| v as u32),
                price_bands: bands(r, "PRICEBAND")?,
            }),
            Some(MnspTable::PeriodOffer) => {
                let trading_date = time_of(r, &["TRADINGDATE", "SETTLEMENTDATE"])?.date();
                // MNSP_BIDOFFERPERIOD's layout (TRADINGDATE, OFFERDATETIME)
                // is always in five-minute periods, even in the archive
                // months before five-minute settlement (from March 2021) that
                // carry it; MNSP_PEROFFER's (SETTLEMENTDATE, OFFERDATE) is in
                // 30-minute periods until then. Decided by the layout, not
                // the key: a reader may key both tables alike (an event
                // record keeps either in one file).
                let (period_from, period_to) = if r.text("TRADINGDATE").is_some() {
                    let p = period(r, "PERIODID")?;
                    (p, p)
                } else {
                    periods(r, trading_date)?
                };
                self.periods.push(MnspPeriodOffer {
                    link_id: required(r, "LINKID")?.into_owned(),
                    trading_date,
                    offer_time: time_of(r, &["OFFERDATETIME", "OFFERDATE"])?,
                    version_no: r.num("VERSIONNO").map(|v| v as u32),
                    period_from,
                    period_to,
                    max_avail: r.num("MAXAVAIL").unwrap_or(0.0),
                    ramp_up_rate: r.num("RAMPUPRATE"),
                    band_avail: bands(r, "BANDAVAIL")?,
                });
            }
            None => {}
        }
        Ok(())
    }

    /// The links of `interconnector` in force at `at`: per link, the latest
    /// effective version.
    pub fn links_of(&self, interconnector: &str, at: NaiveDateTime) -> Vec<&MnspLink> {
        let mut best: HashMap<&str, &MnspLink> = HashMap::new();
        for l in self
            .links
            .iter()
            .filter(|l| l.interconnector_id == interconnector && l.effective <= at)
        {
            let e = best.entry(l.link_id.as_str()).or_insert(l);
            if (l.effective, l.version_no) > (e.effective, e.version_no) {
                *e = l;
            }
        }
        let mut out: Vec<&MnspLink> = best.into_values().collect();
        out.sort_by(|a, b| a.link_id.cmp(&b.link_id));
        out
    }

    /// The offer in force for `link` in the interval ending `interval_end`:
    /// the latest period row covering its period submitted at or before
    /// `cutoff`, and the day offer of the same version if present (the
    /// same trading day where there is one, else any: a standing offer's
    /// prices are under the day it was submitted for).
    pub fn effective(
        &self,
        link: &str,
        interval_end: NaiveDateTime,
        cutoff: NaiveDateTime,
    ) -> Option<(Option<&MnspDayOffer>, &MnspPeriodOffer)> {
        let (day, period) = period_of(interval_end);
        let p = self
            .periods
            .iter()
            .filter(|p| {
                p.link_id == link
                    && p.trading_date == day
                    && p.covers(period)
                    && p.offer_time <= cutoff
            })
            .max_by_key(|p| (p.offer_time, p.version_no))?;
        let same = |d: &&MnspDayOffer| {
            d.link_id == link
                && d.offer_time == p.offer_time
                && (p.version_no.is_none()
                    || d.version_no.is_none()
                    || d.version_no == p.version_no)
        };
        let d = self
            .day
            .iter()
            .filter(same)
            .find(|d| d.trading_date == day)
            .or_else(|| self.day.iter().find(same));
        Some((d, p))
    }

    /// The bands of `link`'s offer in force (see [`MnspOffers::effective`]),
    /// cut off at MAXAVAIL, as (MW, $/MWh) with no empty bands; `None` if
    /// there is no offer or its prices are missing.
    pub fn bands(
        &self,
        link: &str,
        interval_end: NaiveDateTime,
        cutoff: NaiveDateTime,
    ) -> Option<Vec<(f64, f64)>> {
        let (d, p) = self.effective(link, interval_end, cutoff)?;
        let d = d?;
        let mut left = p.max_avail.max(0.0);
        let mut out = Vec::new();
        for (mw, price) in p.band_avail.iter().zip(d.price_bands) {
            let mw = mw.min(left);
            left -= mw;
            if mw > 0.0 {
                out.push((mw, price));
            }
        }
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::visit_report;
    use crate::time::parse_market;

    fn t(s: &str) -> NaiveDateTime {
        parse_market(s).unwrap()
    }

    // Basslink on 30 Jan 2020, before five-minute settlement: the standing
    // offer submitted for 29 Jan (prices under that day; AEMO's figures),
    // period 29 is 18:00-18:30. Illustrative, not AEMO's: the version of
    // 18:22 (not yet in force at 18:20) and the 2018 link row.
    const OLD: &str = "\
I,PARTICIPANT_REGISTRATION,MNSP_INTERCONNECTOR,2,LINKID,EFFECTIVEDATE,VERSIONNO,INTERCONNECTORID,FROMREGION,TOREGION,MAXCAPACITY
D,PARTICIPANT_REGISTRATION,MNSP_INTERCONNECTOR,2,BLNKTAS,2019/07/01 00:00:00,1,T-V-MNSP1,TAS1,VIC1,594
D,PARTICIPANT_REGISTRATION,MNSP_INTERCONNECTOR,2,BLNKVIC,2019/07/01 00:00:00,1,T-V-MNSP1,VIC1,TAS1,478
D,PARTICIPANT_REGISTRATION,MNSP_INTERCONNECTOR,2,BLNKTAS,2018/07/01 00:00:00,1,T-V-MNSP1,TAS1,VIC1,500
I,MNSP,DAYOFFER,1,SETTLEMENTDATE,OFFERDATE,VERSIONNO,LINKID,PRICEBAND1,PRICEBAND2,PRICEBAND3,PRICEBAND4,PRICEBAND5,PRICEBAND6,PRICEBAND7,PRICEBAND8,PRICEBAND9,PRICEBAND10
D,MNSP,DAYOFFER,1,2020/01/29 00:00:00,2020/01/29 00:00:00,1,BLNKTAS,-972.8,-299.99,-49.99,0.01,50.01,100.01,300.01,1000.01,5000.01,14700
I,MNSP,PEROFFER,1,SETTLEMENTDATE,OFFERDATE,VERSIONNO,LINKID,PERIODID,MAXAVAIL,BANDAVAIL1,BANDAVAIL2,BANDAVAIL3,BANDAVAIL4,BANDAVAIL5,BANDAVAIL6,BANDAVAIL7,BANDAVAIL8,BANDAVAIL9,BANDAVAIL10,RAMPUPRATE
D,MNSP,PEROFFER,1,2020/01/30 00:00:00,2020/01/29 00:00:00,1,BLNKTAS,29,478,0,0,0,445,0,0,0,0,0,149,200
D,MNSP,PEROFFER,1,2020/01/30 00:00:00,2020/01/30 18:22:00,2,BLNKTAS,29,100,0,0,0,100,0,0,0,0,0,0,200
";

    #[test]
    fn reads_links_and_half_hour_offers() {
        let mut o = MnspOffers::default();
        visit_report(OLD.as_bytes(), |r| o.push(r)).unwrap();
        assert_eq!((o.links.len(), o.day.len(), o.periods.len()), (3, 1, 2));
        let p = &o.periods[0];
        assert_eq!((p.period_from, p.period_to), (169, 174));
        let links = o.links_of("T-V-MNSP1", t("2020/01/30 18:20:00"));
        let caps: Vec<_> = links
            .iter()
            .map(|l| (l.link_id.as_str(), l.max_capacity))
            .collect();
        assert_eq!(caps, [("BLNKTAS", Some(594.0)), ("BLNKVIC", Some(478.0))]);
    }

    #[test]
    fn half_hour_layout_read_under_the_five_minute_key() {
        // An event record keeps MNSP_PEROFFER rows in the file read as
        // MNSP_BIDOFFERPERIOD; their periods are still half hours.
        let old = OLD
            .replace("I,MNSP,PEROFFER", "I,BIDS,MNSP_BIDOFFERPERIOD")
            .replace("D,MNSP,PEROFFER", "D,BIDS,MNSP_BIDOFFERPERIOD");
        let mut o = MnspOffers::default();
        visit_report(old.as_bytes(), |r| o.push(r)).unwrap();
        assert_eq!(o.periods.len(), 2);
        let p = &o.periods[0];
        assert_eq!((p.period_from, p.period_to), (169, 174));
    }

    #[test]
    fn standing_offer_in_force_and_bands_cut_at_maxavail() {
        let mut o = MnspOffers::default();
        visit_report(OLD.as_bytes(), |r| o.push(r)).unwrap();
        let end = t("2020/01/30 18:20:00");
        let (d, p) = o
            .effective("BLNKTAS", end, t("2020/01/30 18:15:00"))
            .unwrap();
        assert_eq!(p.max_avail, 478.0);
        assert_eq!(
            d.unwrap().trading_date,
            NaiveDate::from_ymd_opt(2020, 1, 29).unwrap()
        );
        // 445 MW at $0.01, then 33 of the 149 at $14,700 (MAXAVAIL 478).
        assert_eq!(
            o.bands("BLNKTAS", end, t("2020/01/30 18:15:00")),
            Some(vec![(445.0, 0.01), (33.0, 14700.0)])
        );
        // The 18:22 version has no day offer here: no prices, no bands.
        assert_eq!(
            o.bands(
                "BLNKTAS",
                t("2020/01/30 18:30:00"),
                t("2020/01/30 18:25:00")
            ),
            None
        );
        // No offer for the other link.
        assert!(o
            .effective("BLNKVIC", end, t("2020/01/30 18:15:00"))
            .is_none());
    }
}
