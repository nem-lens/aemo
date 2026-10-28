//! NEMDE case files: the dispatch engine's inputs and solution, per interval.
//!
//! AEMO archives every dispatch run of NEMDE as an XML case file, one per
//! interval, zipped by trading day:
//! `NemSpdOutputs_<YYYYMMDD>_loaded.zip` in the NEMDE archive (see
//! [`day_zip_url`]), up to the last full month. Despite the name, each file
//! holds the whole case: what NEMDE was given and what it returned.
//!
//! - **Inputs** (`NemSpdInputs`): penalty prices ([`Case`]); units with
//!   initial conditions and band prices ([`Trader`]); interconnectors with
//!   loss models and MNSP band prices ([`Interconnector`]); SCADA values
//!   ([`ScadaValue`]); every invoked generic constraint with its LHS factors,
//!   RHS terms and RHS ([`GenericConstraint`]); and per period, demand,
//!   band availability, limits and entered values ([`Period`]).
//! - **Outputs** (`NemSpdOutputs`): region prices and dispatch, flows and
//!   losses, unit targets in every market, and each constraint's RHS,
//!   marginal value and deficit ([`Solution`]).
//! - **Price setting** (`SolutionAnalysis`): the bands that set each price,
//!   as in the separate `NemPriceSetter` files ([`PriceSetting`]).
//!
//! # Prices are referred
//!
//! Energy band prices (`ENOF`, `LDOF`, `BDOF`) in [`TradePrices`] are already
//! referred to the regional reference node: the offer price divided by the
//! unit's loss factor, rounded to cents. ADPBA1's bands on 10 Jul 2025 were
//! −980, 0 and 54 $/MWh in its offer and −946.85, 0 and 52.17 in the case,
//! a factor of 1/1.03502 = 1/(TLF 1.0037 × DLF 1.0312) in both directions.
//! FCAS band prices are as offered. Only that unit has been checked.
//!
//! # Schema across years
//!
//! The format is `VERSION="2.1.0"` from 2020 to 2025 and grows by adding
//! attributes: `Direction` on trades and prices for bidirectional units
//! (from June 2024), 1-second FCAS, energy storage, administered pricing
//! events. Each struct types the fields that have been seen in every year
//! checked (Jan 2020, Jun 2022, Jul 2025) and keeps every other attribute
//! as text in `other`, so nothing is dropped. Elements this module does not
//! read are listed in [`CaseFile::unread`].
//!
//! # Example
//!
//! ```
//! use nem_lens_aemo::nemde::read_case;
//!
//! # fn main() -> anyhow::Result<()> {
//! let xml = r#"<?xml version="1.0" encoding="utf-8"?>
//! <NEMSPDCaseFile><NemSpdInputs VERSION="2.1.0">
//!   <Case CaseID="20250710018" CaseType="DS" Intervention="False" VoLL="20300" MPF="-1000" TieBreakPrice="1E-6"/>
//! </NemSpdInputs><NemSpdOutputs>
//!   <RegionSolution RegionID="NSW1" PeriodID="2025-07-10T05:30:00+10:00" Intervention="0" EnergyPrice="77.30009" NetExport="-262.64" R6Price="0.29"/>
//! </NemSpdOutputs></NEMSPDCaseFile>"#;
//! let case = read_case(xml.as_bytes())?;
//! assert_eq!(case.case.prices()["TieBreakPrice"], 1e-6);
//! let nsw = &case.solution.regions[0];
//! assert_eq!(nsw.energy_price, 77.30009);
//! assert_eq!(nsw.value("R6Price"), Some(0.29));
//! # Ok(())
//! # }
//! ```

use std::collections::{BTreeMap, BTreeSet};
use std::io::{Cursor, Read};

use anyhow::{anyhow, bail, Context, Result};
use chrono::{DateTime, Duration, NaiveDate, NaiveDateTime};
use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, XmlVersion};

/// Attributes not typed by a struct, as text, by name.
pub type Attrs = BTreeMap<String, String>;

/// The NEMDE archive's URL for a trading day's case files.
pub fn day_zip_url(day: NaiveDate) -> String {
    format!(
        "https://nemweb.com.au/Data_Archive/Wholesale_Electricity/NEMDE/{y}/NEMDE_{y}_{m}/NEMDE_Market_Data/NEMDE_Files/NemSpdOutputs_{d}_loaded.zip",
        y = day.format("%Y"),
        m = day.format("%m"),
        d = day.format("%Y%m%d"),
    )
}

/// What a case file's name says about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaseName {
    /// The trading day (04:05 to 04:00 the next day).
    pub day: NaiveDate,
    /// The dispatch interval of the day, 1 to 288.
    pub period: u32,
    /// The interval's end, market time.
    pub interval_end: NaiveDateTime,
    /// The rest of the name: "00" for the dispatch run, or a re-run's tag
    /// such as "_MANUALOCD_TOM00_OCD" (a manual over-constrained dispatch
    /// re-run, seen on 13 Jun 2022).
    pub run: String,
}

impl CaseName {
    /// True for the ordinary dispatch run, false for re-runs.
    pub fn is_dispatch_run(&self) -> bool {
        self.run == "00"
    }
}

/// Parse a member name such as `NEMSPDOutputs_2025071001800.loaded`.
pub fn parse_case_name(name: &str) -> Option<CaseName> {
    let base = name.rsplit('/').next()?;
    let stem = base
        .strip_prefix("NEMSPDOutputs_")?
        .strip_suffix(".loaded")?;
    let day = NaiveDate::parse_from_str(stem.get(..8)?, "%Y%m%d").ok()?;
    let period: u32 = stem.get(8..11)?.parse().ok()?;
    if !(1..=288).contains(&period) {
        return None;
    }
    let interval_end = day.and_hms_opt(4, 0, 0)? + Duration::minutes(5 * i64::from(period));
    Some(CaseName {
        day,
        period,
        interval_end,
        run: stem[11..].to_string(),
    })
}

/// Each member of a day's zip, as (name, uncompressed size), in archive order.
pub fn day_members(zip: &[u8]) -> Result<Vec<(String, u64)>> {
    let mut archive = zip::ZipArchive::new(Cursor::new(zip)).context("not a zip archive")?;
    (0..archive.len())
        .map(|i| {
            let f = archive.by_index(i)?;
            Ok((f.name()?.to_string(), f.size()))
        })
        .collect()
}

/// One member of a day's zip, decompressed. Fails if it holds more than
/// `max_bytes` (a case is about 7 to 13 MB).
pub fn day_member(zip: &[u8], name: &str, max_bytes: u64) -> Result<Vec<u8>> {
    let mut archive = zip::ZipArchive::new(Cursor::new(zip)).context("not a zip archive")?;
    let f = archive
        .by_name(name)
        .with_context(|| format!("no member {name}"))?;
    let mut buf = Vec::new();
    f.take(max_bytes + 1).read_to_end(&mut buf)?;
    if buf.len() as u64 > max_bytes {
        bail!("{name} is larger than {max_bytes} bytes");
    }
    Ok(buf)
}

/// A whole case file.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CaseFile {
    /// `NemSpdInputs` VERSION, e.g. "2.1.0".
    pub version: String,
    /// The case and its penalty prices.
    pub case: Case,
    /// Administered pricing events in force (`APEvent_Item`).
    pub ap_events: Vec<Attrs>,
    /// Regions and their initial conditions.
    pub regions: Vec<Region>,
    /// Units, with initial conditions and band prices.
    pub traders: Vec<Trader>,
    /// Interconnectors, with loss models and MNSP prices.
    pub interconnectors: Vec<Interconnector>,
    /// SCADA values that constraint RHS terms read, good and bad.
    pub scada: Vec<ScadaValue>,
    /// Generic equations, shared RHS terms that constraints refer to.
    pub equations: Vec<GenericEquation>,
    /// Every generic constraint in the case.
    pub constraints: Vec<GenericConstraint>,
    /// The periods solved: one in a dispatch case.
    pub periods: Vec<Period>,
    /// NEMDE's solution.
    pub solution: Solution,
    /// The bands that set each price.
    pub price_setting: Vec<PriceSetting>,
    /// In a manual over-constrained dispatch (OCD) re-run case, the price
    /// setters of the original run (`PriceSetting_Original`).
    pub price_setting_original: Vec<PriceSetting>,
    /// In an OCD re-run case, what triggered it (`Violating_GenericConstraint`,
    /// e.g. Violation_Type NETWORK).
    pub ocd_violations: Vec<Attrs>,
    /// Elements with attributes that this module does not read, by name.
    pub unread: BTreeSet<String>,
}

/// The case (`Case`): its identity and penalty prices.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Case {
    /// CaseID, e.g. "20250710018" (trading day and period).
    pub case_id: String,
    /// CaseType, e.g. "DS" (dispatch).
    pub case_type: String,
    /// Intervention: whether this is an intervention case.
    pub intervention: bool,
    /// VoLL: the market price cap, $/MWh.
    pub voll: f64,
    /// MPF: the market floor price, $/MWh.
    pub mpf: f64,
    /// Every other attribute, including the penalty prices.
    pub other: Attrs,
}

impl Case {
    /// Every attribute whose name ends in "Price", as a number: the
    /// constraint violation penalties (EnergyDeficitPrice, RampRatePrice,
    /// GenericConstraintPrice, ...), OfferPrice, TieBreakPrice and so on.
    pub fn prices(&self) -> BTreeMap<String, f64> {
        self.other
            .iter()
            .filter(|(k, _)| k.ends_with("Price"))
            .filter_map(|(k, v)| Some((k.clone(), v.parse().ok()?)))
            .collect()
    }
}

/// A region (`Region`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Region {
    /// RegionID, e.g. "NSW1".
    pub region_id: String,
    /// RegionInitialCondition values by InitialConditionID (e.g. "ADE").
    pub initial_conditions: BTreeMap<String, f64>,
    /// Administered pricing flags by event (`APEvent_Region_Item`).
    pub ap_events: Vec<Attrs>,
    /// Every other attribute.
    pub other: Attrs,
}

/// A unit (`Trader`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Trader {
    /// TraderID: the DUID.
    pub trader_id: String,
    /// TraderType: GENERATOR, LOAD, WDR or BIDIRECTIONAL.
    pub trader_type: String,
    /// SemiDispatch.
    pub semi_dispatch: bool,
    /// TraderInitialCondition values by InitialConditionID (InitialMW,
    /// AGCStatus, HMW, LMW, SCADARampUpRate, ...).
    pub initial_conditions: BTreeMap<String, f64>,
    /// Band prices by trade type and direction.
    pub prices: Vec<TradePrices>,
    /// Every other attribute (FastStart, MinLoadingMW, CurrentMode, T1 to
    /// T4, ...).
    pub other: Attrs,
}

/// One trade type's band prices (`TradeTypePriceStructure`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TradePrices {
    /// The TradePriceStructureID it belongs to; [`TraderPeriod`] names the
    /// one in force.
    pub structure_id: String,
    /// TradeType: ENOF (energy), LDOF (load), BDOF (bidirectional), or a
    /// FCAS market (R6SE, R60S, R5MI, R5RE, L6SE, L60S, L5MI, L5RE, R1SE,
    /// L1SE).
    pub trade_type: String,
    /// Direction: GEN or LOAD, for bidirectional units (from 2024).
    pub direction: Option<String>,
    /// PriceBand1 to 10, $/MWh. Energy bands are referred to the regional
    /// node (see the module docs); FCAS bands are as offered.
    pub price_bands: [f64; 10],
    /// Every other attribute (Offer_SettlementDate, Offer_EffectiveDate,
    /// Offer_VersionNo).
    pub other: Attrs,
}

/// An interconnector (`Interconnector`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Interconnector {
    /// InterconnectorID, e.g. "VIC1-NSW1".
    pub interconnector_id: String,
    /// InterconnectorInitialCondition values (InitialMW, WhatIfInitialMW).
    pub initial_conditions: BTreeMap<String, f64>,
    /// Its loss models.
    pub loss_models: Vec<LossModel>,
    /// MNSP band prices by link, for an MNSP (Basslink).
    pub mnsp_prices: Vec<MnspPrices>,
    /// Every other attribute (FromRegionLF, ToRegionLF and their import
    /// and export variants).
    pub other: Attrs,
}

/// An interconnector loss model (`LossModel`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LossModel {
    /// LossModelID; [`InterconnectorPeriod::loss_model_id`] names the one
    /// in force.
    pub loss_model_id: String,
    /// LossLowerLimit, MW.
    pub loss_lower_limit: f64,
    /// LossShare: the share of losses assigned to the from region.
    pub loss_share: f64,
    /// Segments, in file order.
    pub segments: Vec<LossSegment>,
    /// Every other attribute (NPLRange).
    pub other: Attrs,
}

/// One segment of a loss model (`Segment`).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct LossSegment {
    /// Limit: the flow, MW, at which the segment ends.
    pub limit: f64,
    /// Factor: the marginal loss factor over the segment.
    pub factor: f64,
}

/// An MNSP link's band prices (`MNSPRegionPriceStructure`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MnspPrices {
    /// The MNSPPriceStructureID it belongs to.
    pub structure_id: String,
    /// LinkID, e.g. "BLNKTAS".
    pub link_id: String,
    /// RegionID: the region the link offers into.
    pub region_id: String,
    /// PriceBand1 to 10, $/MWh.
    pub price_bands: [f64; 10],
    /// Every other attribute (ParticipantID, Offer_*).
    pub other: Attrs,
}

/// A SCADA value (`ScadaValues`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ScadaValue {
    /// The SpdType of its `ConstraintScadaData` group (A, C, E, ...).
    pub spd_type: String,
    /// SpdID, as RHS terms name it.
    pub spd_id: String,
    /// Value.
    pub value: f64,
    /// True if listed under `BadScadaValuesCollection`.
    pub bad: bool,
    /// Every other attribute (EMS_ID, EMS_Key, quality flags, ...).
    pub other: Attrs,
}

/// A term of a constraint's or equation's right-hand side (`RHSTerm`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RhsTerm {
    /// TermID.
    pub term_id: String,
    /// SpdID: the input it reads.
    pub spd_id: String,
    /// SpdType: the kind of input.
    pub spd_type: String,
    /// Multiplier.
    pub multiplier: f64,
    /// Operation (PUSH, ADD, STEP, ...), where given.
    pub operation: Option<String>,
    /// Default, where given.
    pub default: Option<f64>,
    /// Every other attribute (GroupTerm, ...).
    pub other: Attrs,
}

/// A generic equation (`GenericEquation`): RHS terms shared by constraints.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GenericEquation {
    /// EquationID.
    pub equation_id: String,
    /// Its terms.
    pub rhs_terms: Vec<RhsTerm>,
    /// Every other attribute (EffectiveDate, VersionNo).
    pub other: Attrs,
}

/// A generic constraint (`GenericConstraint`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GenericConstraint {
    /// ConstraintID.
    pub constraint_id: String,
    /// Version, e.g. "20241004000000_1".
    pub version: String,
    /// Type: LE, GE or EQ.
    pub constraint_type: String,
    /// RHS: the value NEMDE used.
    pub rhs: f64,
    /// ViolationPrice: the cost of violating it by 1 MW.
    pub violation_price: f64,
    /// Unit terms of the LHS (`TraderFactor`).
    pub trader_factors: Vec<TraderFactor>,
    /// Interconnector terms of the LHS (`InterconnectorFactor`).
    pub interconnector_factors: Vec<InterconnectorFactor>,
    /// Region terms of the LHS (`RegionFactor`), e.g. FCAS requirements.
    pub region_factors: Vec<RegionFactor>,
    /// RHS terms, where the RHS is computed from them.
    pub rhs_terms: Vec<RhsTerm>,
    /// Invocation records (`ConstraintTrkItem`).
    pub tracking: Vec<Attrs>,
    /// Every other attribute (EffectiveDate, VersionNo, Force_SCADA).
    pub other: Attrs,
}

/// A unit term of a constraint's LHS.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TraderFactor {
    /// TraderID.
    pub trader_id: String,
    /// TradeType: the market the term applies to.
    pub trade_type: String,
    /// Factor.
    pub factor: f64,
}

/// An interconnector term of a constraint's LHS.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct InterconnectorFactor {
    /// InterconnectorID.
    pub interconnector_id: String,
    /// Factor.
    pub factor: f64,
}

/// A region term of a constraint's LHS.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RegionFactor {
    /// RegionID.
    pub region_id: String,
    /// TradeType: the market the term applies to.
    pub trade_type: String,
    /// Factor.
    pub factor: f64,
}

/// One period's inputs (`Period`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Period {
    /// PeriodID: the interval's end, market time.
    pub period_id: NaiveDateTime,
    /// EnteredValuePeriod values by SpdID (operator-entered RHS inputs).
    pub entered_values: BTreeMap<String, f64>,
    /// Region inputs.
    pub regions: Vec<RegionPeriod>,
    /// Unit inputs: band availability and limits.
    pub traders: Vec<TraderPeriod>,
    /// Interconnector inputs: limits, loss model, MNSP offers.
    pub interconnectors: Vec<InterconnectorPeriod>,
    /// Which constraints apply (`GenericConstraintPeriod`).
    pub constraints: Vec<ConstraintPeriod>,
    /// Non-scheduled generation by DUID, MW.
    pub non_scheduled: BTreeMap<String, f64>,
}

/// A region's inputs for a period (`RegionPeriod`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RegionPeriod {
    /// RegionID.
    pub region_id: String,
    /// DemandForecast, MW.
    pub demand_forecast: f64,
    /// RegionPeriodUsageFactor values by TradeType (from 2024).
    pub usage_factors: BTreeMap<String, f64>,
    /// Every other attribute (DF, suspension schedule prices, ...).
    pub other: Attrs,
}

/// A unit's inputs for a period (`TraderPeriod`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TraderPeriod {
    /// TraderID.
    pub trader_id: String,
    /// RegionID.
    pub region_id: String,
    /// TradePriceStructureID: which of the unit's [`TradePrices`] apply.
    pub structure_id: String,
    /// UIGF, MW, for semi-scheduled units.
    pub uigf: Option<f64>,
    /// Its trades, one per trade type and direction.
    pub trades: Vec<Trade>,
    /// Every other attribute (storage and efficiency fields, ...).
    pub other: Attrs,
}

/// One trade type's quantities for a period (`Trade`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Trade {
    /// TradeType, as in [`TradePrices::trade_type`].
    pub trade_type: String,
    /// Direction: GEN or LOAD, for bidirectional units (from 2024).
    pub direction: Option<String>,
    /// BandAvail1 to 10, MW.
    pub band_avail: [f64; 10],
    /// MaxAvail, MW.
    pub max_avail: f64,
    /// RampUpRate, MW/h, where given.
    pub ramp_up_rate: Option<f64>,
    /// RampDnRate, MW/h, where given.
    pub ramp_dn_rate: Option<f64>,
    /// EnablementMin, MW (FCAS trapezium).
    pub enablement_min: Option<f64>,
    /// LowBreakpoint, MW.
    pub low_breakpoint: Option<f64>,
    /// HighBreakpoint, MW.
    pub high_breakpoint: Option<f64>,
    /// EnablementMax, MW.
    pub enablement_max: Option<f64>,
    /// Every other attribute.
    pub other: Attrs,
}

/// An interconnector's inputs for a period (`InterconnectorPeriod`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct InterconnectorPeriod {
    /// InterconnectorID.
    pub interconnector_id: String,
    /// FromRegion.
    pub from_region: String,
    /// ToRegion.
    pub to_region: String,
    /// LowerLimit, MW: the limit on flow from ToRegion to FromRegion.
    pub lower_limit: f64,
    /// UpperLimit, MW: the limit on flow from FromRegion to ToRegion.
    pub upper_limit: f64,
    /// MNSP: whether it is an MNSP.
    pub mnsp: bool,
    /// LossModelID: which of the interconnector's [`LossModel`]s applies.
    pub loss_model_id: String,
    /// MNSP offers, one per link direction.
    pub mnsp_offers: Vec<MnspOffer>,
    /// Every other attribute (loss factors, LossDemandConstant, ...).
    pub other: Attrs,
}

/// An MNSP link's quantities for a period (`MNSPOffer`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MnspOffer {
    /// RegionID: the region the link offers into.
    pub region_id: String,
    /// BandAvail1 to 10, MW.
    pub band_avail: [f64; 10],
    /// MaxAvail, MW.
    pub max_avail: f64,
    /// Every other attribute (RampUpRate, RampDnRate).
    pub other: Attrs,
}

/// A constraint applying in a period (`GenericConstraintPeriod`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ConstraintPeriod {
    /// ConstraintID.
    pub constraint_id: String,
    /// Version.
    pub version: String,
    /// Every other attribute (Intervention, Category).
    pub other: Attrs,
}

/// NEMDE's solution (`NemSpdOutputs`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Solution {
    /// CaseSolution: solver status, objective, violation totals.
    pub case: Attrs,
    /// PeriodSolution, one per period and run.
    pub periods: Vec<Attrs>,
    /// Region results.
    pub regions: Vec<RegionSolution>,
    /// Interconnector results.
    pub interconnectors: Vec<InterconnectorSolution>,
    /// Unit results.
    pub traders: Vec<TraderSolution>,
    /// Constraint results.
    pub constraints: Vec<ConstraintSolution>,
    /// In a manual OCD re-run case, the original run's region results
    /// (`RegionSolution_Original`); `regions` then holds the re-run's.
    pub regions_original: Vec<RegionSolution>,
    /// In a manual OCD re-run case, FCAS constraint results
    /// (`ConstraintSolution_FCAS_OCD`).
    pub fcas_ocd_constraints: Vec<FcasOcdConstraint>,
}

/// A FCAS constraint's result in an OCD re-run case
/// (`ConstraintSolution_FCAS_OCD`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FcasOcdConstraint {
    /// ConstraintID.
    pub constraint_id: String,
    /// Intervention.
    pub intervention: bool,
    /// RHS.
    pub rhs: f64,
    /// MarginalValue.
    pub marginal_value: f64,
    /// ViolationDegree, MW.
    pub violation_degree: f64,
    /// Every other attribute.
    pub other: Attrs,
}

/// A region's result (`RegionSolution`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RegionSolution {
    /// RegionID.
    pub region_id: String,
    /// PeriodID.
    pub period_id: NaiveDateTime,
    /// Intervention: true for an intervention (what-if) run's result.
    pub intervention: bool,
    /// EnergyPrice, $/MWh.
    pub energy_price: f64,
    /// NetExport, MW.
    pub net_export: f64,
    /// Every other attribute: FixedDemand, FCAS prices and dispatch
    /// (R6Price, R6Dispatch, ...), and more by year.
    pub other: Attrs,
}

/// An interconnector's result (`InterconnectorSolution`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct InterconnectorSolution {
    /// InterconnectorID.
    pub interconnector_id: String,
    /// PeriodID.
    pub period_id: NaiveDateTime,
    /// Intervention.
    pub intervention: bool,
    /// Flow, MW, positive from FromRegion to ToRegion.
    pub flow: f64,
    /// Losses, MW.
    pub losses: f64,
    /// InterRegionalLossFactor, where given.
    pub inter_regional_loss_factor: Option<f64>,
    /// Every other attribute (Deficit, IdealLosses, NPLExists, ...).
    pub other: Attrs,
}

/// A unit's result (`TraderSolution`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TraderSolution {
    /// TraderID.
    pub trader_id: String,
    /// PeriodID.
    pub period_id: NaiveDateTime,
    /// Intervention.
    pub intervention: bool,
    /// EnergyTarget, MW (negative when charging).
    pub energy_target: f64,
    /// Every other attribute: FCAS targets (R6Target, ...), flags,
    /// violations and more.
    pub other: Attrs,
}

/// A constraint's result (`ConstraintSolution`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ConstraintSolution {
    /// ConstraintID.
    pub constraint_id: String,
    /// Version.
    pub version: String,
    /// PeriodID.
    pub period_id: NaiveDateTime,
    /// Intervention.
    pub intervention: bool,
    /// RHS.
    pub rhs: f64,
    /// MarginalValue.
    pub marginal_value: f64,
    /// Deficit: the violation, MW.
    pub deficit: f64,
    /// Every other attribute.
    pub other: Attrs,
}

/// One band's part in setting a price (`PriceSetting`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PriceSetting {
    /// PeriodID.
    pub period_id: NaiveDateTime,
    /// RegionID.
    pub region_id: String,
    /// Market: Energy or a FCAS market.
    pub market: String,
    /// Price: the region's price in that market.
    pub price: f64,
    /// Unit: a DUID, an interconnector or link, or a tie-break pair.
    pub unit: String,
    /// DispatchedMarket: the market of the band (ENOF, R6SE, ...).
    pub dispatched_market: String,
    /// BandNo, as text: a band number ("1" to "10") for an offer band; the
    /// constraint's version (e.g. "20220419000000_1") where a constraint's
    /// violation sets the price (DispatchedMarket GenericConstraintSurplus,
    /// Unit the ConstraintID); absent for NEMDE's tie-break variables
    /// (DispatchedMarket tbslack1 or tbslack2, Unit naming the tied pair).
    pub band_no: Option<String>,
    /// Increase: the change in the band's dispatch per MW of the region's
    /// requirement.
    pub increase: f64,
    /// RRNBandPrice: the band's price referred to the regional node.
    pub rrn_band_price: f64,
    /// BandCost: RRNBandPrice × Increase.
    pub band_cost: f64,
}

macro_rules! value_of_other {
    ($($t:ty),*) => {$(
        impl $t {
            /// An attribute kept in `other`, as a number.
            pub fn value(&self, name: &str) -> Option<f64> {
                self.other.get(name)?.parse().ok()
            }
        }
    )*};
}
value_of_other!(
    Region,
    Trader,
    Interconnector,
    RegionPeriod,
    TraderPeriod,
    Trade,
    InterconnectorPeriod,
    RegionSolution,
    InterconnectorSolution,
    TraderSolution,
    ConstraintSolution
);

/// Read a case file's XML.
pub fn read_case(xml: &[u8]) -> Result<CaseFile> {
    let text = std::str::from_utf8(xml).context("case file is not UTF-8")?;
    let mut reader = Reader::from_str(text);
    let mut out = CaseFile::default();
    let mut stack: Vec<String> = Vec::new();
    let mut price_structure = String::new();
    let mut mnsp_structure = String::new();
    let mut scada_type = String::new();
    loop {
        match reader.read_event()? {
            Event::Start(e) => {
                let name = local_name(&e)?;
                element(
                    &mut out,
                    &stack,
                    &name,
                    &e,
                    &mut price_structure,
                    &mut mnsp_structure,
                    &mut scada_type,
                )
                .with_context(|| format!("<{name}> at byte {}", reader.buffer_position()))?;
                stack.push(name);
            }
            Event::Empty(e) => {
                let name = local_name(&e)?;
                element(
                    &mut out,
                    &stack,
                    &name,
                    &e,
                    &mut price_structure,
                    &mut mnsp_structure,
                    &mut scada_type,
                )
                .with_context(|| format!("<{name}> at byte {}", reader.buffer_position()))?;
            }
            Event::End(_) => {
                stack.pop();
            }
            Event::Eof => break,
            _ => {}
        }
    }
    if out.case.case_id.is_empty() {
        bail!("no <Case> element: not a NEMDE case file");
    }
    Ok(out)
}

fn local_name(e: &BytesStart) -> Result<String> {
    Ok(std::str::from_utf8(e.local_name().as_ref())?.to_string())
}

/// The attributes of an element, taken by name as they are typed; what is
/// left goes to `other`.
struct A(Attrs);

impl A {
    fn of(e: &BytesStart) -> Result<A> {
        let mut m = Attrs::new();
        for a in e.attributes() {
            let a = a?;
            if a.key.as_namespace_binding().is_some() {
                continue;
            }
            let k = std::str::from_utf8(a.key.local_name().as_ref())?.to_string();
            m.insert(k, a.normalized_value(XmlVersion::Explicit1_0)?.into_owned());
        }
        Ok(A(m))
    }
    fn s(&mut self, k: &str) -> Result<String> {
        self.0.remove(k).ok_or_else(|| anyhow!("missing {k}"))
    }
    fn os(&mut self, k: &str) -> Option<String> {
        self.0.remove(k)
    }
    fn f(&mut self, k: &str) -> Result<f64> {
        let v = self.s(k)?;
        v.parse()
            .with_context(|| format!("{k}={v:?} is not a number"))
    }
    fn of64(&mut self, k: &str) -> Result<Option<f64>> {
        match self.0.remove(k) {
            None => Ok(None),
            Some(v) => Ok(Some(
                v.parse()
                    .with_context(|| format!("{k}={v:?} is not a number"))?,
            )),
        }
    }
    fn b(&mut self, k: &str) -> Result<bool> {
        let v = self.s(k)?;
        match v.as_str() {
            "True" | "true" | "1" => Ok(true),
            "False" | "false" | "0" => Ok(false),
            _ => bail!("{k}={v:?} is not a flag"),
        }
    }
    fn ob(&mut self, k: &str) -> Result<bool> {
        if self.0.contains_key(k) {
            self.b(k)
        } else {
            Ok(false)
        }
    }
    fn t(&mut self, k: &str) -> Result<NaiveDateTime> {
        let v = self.s(k)?;
        Ok(DateTime::parse_from_rfc3339(&v)
            .with_context(|| format!("{k}={v:?} is not a time"))?
            .naive_local())
    }
    fn bands(&mut self, prefix: &str) -> Result<[f64; 10]> {
        let mut b = [0.0; 10];
        for (i, x) in b.iter_mut().enumerate() {
            *x = self.f(&format!("{prefix}{}", i + 1))?;
        }
        Ok(b)
    }
    fn id_value(mut self) -> Result<(String, f64)> {
        Ok((self.s("InitialConditionID")?, self.f("Value")?))
    }
}

fn last<'a, T>(v: &'a mut [T], what: &str) -> Result<&'a mut T> {
    v.last_mut().ok_or_else(|| anyhow!("outside a {what}"))
}

#[allow(clippy::too_many_arguments)]
fn element(
    out: &mut CaseFile,
    stack: &[String],
    name: &str,
    e: &BytesStart,
    price_structure: &mut String,
    mnsp_structure: &mut String,
    scada_type: &mut String,
) -> Result<()> {
    let mut a = A::of(e)?;
    let parent = stack.last().map(String::as_str).unwrap_or("");
    let grandparent = stack.len().checked_sub(2).map_or("", |i| stack[i].as_str());
    match name {
        "NemSpdInputs" => out.version = a.os("VERSION").unwrap_or_default(),
        "Case" => {
            out.case = Case {
                case_id: a.s("CaseID")?,
                case_type: a.s("CaseType")?,
                intervention: a.b("Intervention")?,
                voll: a.f("VoLL")?,
                mpf: a.f("MPF")?,
                other: a.0,
            }
        }
        "APEvent_Item" => out.ap_events.push(a.0),
        "Region" => out.regions.push(Region {
            region_id: a.s("RegionID")?,
            other: a.0,
            ..Default::default()
        }),
        "RegionInitialCondition" => {
            let (k, v) = a.id_value()?;
            last(&mut out.regions, "Region")?
                .initial_conditions
                .insert(k, v);
        }
        "APEvent_Region_Item" => last(&mut out.regions, "Region")?.ap_events.push(a.0),
        "Trader" => out.traders.push(Trader {
            trader_id: a.s("TraderID")?,
            trader_type: a.s("TraderType")?,
            semi_dispatch: a.ob("SemiDispatch")?,
            other: a.0,
            ..Default::default()
        }),
        "TraderInitialCondition" => {
            let (k, v) = a.id_value()?;
            last(&mut out.traders, "Trader")?
                .initial_conditions
                .insert(k, v);
        }
        "TradePriceStructure" => *price_structure = a.s("TradePriceStructureID")?,
        "TradeTypePriceStructure" => {
            let p = TradePrices {
                structure_id: price_structure.clone(),
                trade_type: a.s("TradeType")?,
                direction: a.os("Direction"),
                price_bands: a.bands("PriceBand")?,
                other: a.0,
            };
            last(&mut out.traders, "Trader")?.prices.push(p);
        }
        "Interconnector" => out.interconnectors.push(Interconnector {
            interconnector_id: a.s("InterconnectorID")?,
            other: a.0,
            ..Default::default()
        }),
        "InterconnectorInitialCondition" => {
            let (k, v) = a.id_value()?;
            last(&mut out.interconnectors, "Interconnector")?
                .initial_conditions
                .insert(k, v);
        }
        "LossModel" => {
            let m = LossModel {
                loss_model_id: a.s("LossModelID")?,
                loss_lower_limit: a.f("LossLowerLimit")?,
                loss_share: a.f("LossShare")?,
                segments: Vec::new(),
                other: a.0,
            };
            last(&mut out.interconnectors, "Interconnector")?
                .loss_models
                .push(m);
        }
        "Segment" => {
            let s = LossSegment {
                limit: a.f("Limit")?,
                factor: a.f("Factor")?,
            };
            last(
                &mut last(&mut out.interconnectors, "Interconnector")?.loss_models,
                "LossModel",
            )?
            .segments
            .push(s);
        }
        "MNSPPriceStructure" => *mnsp_structure = a.s("MNSPPriceStructureID")?,
        "MNSPRegionPriceStructure" => {
            let p = MnspPrices {
                structure_id: mnsp_structure.clone(),
                link_id: a.s("LinkID")?,
                region_id: a.s("RegionID")?,
                price_bands: a.bands("PriceBand")?,
                other: a.0,
            };
            last(&mut out.interconnectors, "Interconnector")?
                .mnsp_prices
                .push(p);
        }
        "ConstraintScadaData" => *scada_type = a.s("SpdType")?,
        "ScadaValues" => out.scada.push(ScadaValue {
            spd_type: scada_type.clone(),
            spd_id: a.s("SpdID")?,
            value: a.f("Value")?,
            bad: parent == "BadScadaValuesCollection",
            other: a.0,
        }),
        "GenericEquation" => out.equations.push(GenericEquation {
            equation_id: a.s("EquationID")?,
            rhs_terms: Vec::new(),
            other: a.0,
        }),
        "RHSTerm" => {
            let t = RhsTerm {
                term_id: a.s("TermID")?,
                spd_id: a.s("SpdID")?,
                spd_type: a.s("SpdType")?,
                multiplier: a.f("Multiplier")?,
                operation: a.os("Operation"),
                default: a.of64("Default")?,
                other: a.0,
            };
            match grandparent {
                "GenericEquation" => last(&mut out.equations, "GenericEquation")?
                    .rhs_terms
                    .push(t),
                "GenericConstraint" => last(&mut out.constraints, "GenericConstraint")?
                    .rhs_terms
                    .push(t),
                _ => bail!("RHSTerm under {grandparent}"),
            }
        }
        "GenericConstraint" => out.constraints.push(GenericConstraint {
            constraint_id: a.s("ConstraintID")?,
            version: a.s("Version")?,
            constraint_type: a.s("Type")?,
            rhs: a.f("RHS")?,
            violation_price: a.f("ViolationPrice")?,
            other: a.0,
            ..Default::default()
        }),
        "TraderFactor" => {
            let f = TraderFactor {
                trader_id: a.s("TraderID")?,
                trade_type: a.s("TradeType")?,
                factor: a.f("Factor")?,
            };
            last(&mut out.constraints, "GenericConstraint")?
                .trader_factors
                .push(f);
        }
        "InterconnectorFactor" => {
            let f = InterconnectorFactor {
                interconnector_id: a.s("InterconnectorID")?,
                factor: a.f("Factor")?,
            };
            last(&mut out.constraints, "GenericConstraint")?
                .interconnector_factors
                .push(f);
        }
        "RegionFactor" => {
            let f = RegionFactor {
                region_id: a.s("RegionID")?,
                trade_type: a.s("TradeType")?,
                factor: a.f("Factor")?,
            };
            last(&mut out.constraints, "GenericConstraint")?
                .region_factors
                .push(f);
        }
        "ConstraintTrkItem" => last(&mut out.constraints, "GenericConstraint")?
            .tracking
            .push(a.0),
        "Period" => out.periods.push(Period {
            period_id: a.t("PeriodID")?,
            ..Default::default()
        }),
        "EnteredValuePeriod" => {
            let (k, v) = (a.s("SpdID")?, a.f("Value")?);
            last(&mut out.periods, "Period")?
                .entered_values
                .insert(k, v);
        }
        "RegionPeriod" => {
            let r = RegionPeriod {
                region_id: a.s("RegionID")?,
                demand_forecast: a.f("DemandForecast")?,
                usage_factors: BTreeMap::new(),
                other: a.0,
            };
            last(&mut out.periods, "Period")?.regions.push(r);
        }
        "RegionPeriodUsageFactor" => {
            let (k, v) = (a.s("TradeType")?, a.f("UsageFactor")?);
            last(
                &mut last(&mut out.periods, "Period")?.regions,
                "RegionPeriod",
            )?
            .usage_factors
            .insert(k, v);
        }
        "TraderPeriod" => {
            let t = TraderPeriod {
                trader_id: a.s("TraderID")?,
                region_id: a.s("RegionID")?,
                structure_id: a.s("TradePriceStructureID")?,
                uigf: a.of64("UIGF")?,
                trades: Vec::new(),
                other: a.0,
            };
            last(&mut out.periods, "Period")?.traders.push(t);
        }
        "Trade" => {
            let t = Trade {
                trade_type: a.s("TradeType")?,
                direction: a.os("Direction"),
                band_avail: a.bands("BandAvail")?,
                max_avail: a.f("MaxAvail")?,
                ramp_up_rate: a.of64("RampUpRate")?,
                ramp_dn_rate: a.of64("RampDnRate")?,
                enablement_min: a.of64("EnablementMin")?,
                low_breakpoint: a.of64("LowBreakpoint")?,
                high_breakpoint: a.of64("HighBreakpoint")?,
                enablement_max: a.of64("EnablementMax")?,
                other: a.0,
            };
            last(
                &mut last(&mut out.periods, "Period")?.traders,
                "TraderPeriod",
            )?
            .trades
            .push(t);
        }
        "InterconnectorPeriod" => {
            let i = InterconnectorPeriod {
                interconnector_id: a.s("InterconnectorID")?,
                from_region: a.s("FromRegion")?,
                to_region: a.s("ToRegion")?,
                lower_limit: a.f("LowerLimit")?,
                upper_limit: a.f("UpperLimit")?,
                mnsp: a.b("MNSP")?,
                loss_model_id: a.s("LossModelID")?,
                mnsp_offers: Vec::new(),
                other: a.0,
            };
            last(&mut out.periods, "Period")?.interconnectors.push(i);
        }
        "MNSPOffer" => {
            let o = MnspOffer {
                region_id: a.s("RegionID")?,
                band_avail: a.bands("BandAvail")?,
                max_avail: a.f("MaxAvail")?,
                other: a.0,
            };
            last(
                &mut last(&mut out.periods, "Period")?.interconnectors,
                "InterconnectorPeriod",
            )?
            .mnsp_offers
            .push(o);
        }
        "GenericConstraintPeriod" => {
            let c = ConstraintPeriod {
                constraint_id: a.s("ConstraintID")?,
                version: a.s("Version")?,
                other: a.0,
            };
            last(&mut out.periods, "Period")?.constraints.push(c);
        }
        "Non_Scheduled_Generator" => {
            let (k, v) = (a.s("DUID")?, a.f("MW")?);
            last(&mut out.periods, "Period")?.non_scheduled.insert(k, v);
        }
        "CaseSolution" => out.solution.case = a.0,
        "PeriodSolution" => out.solution.periods.push(a.0),
        "RegionSolution" => out.solution.regions.push(region_solution(a)?),
        "RegionSolution_Original" => out.solution.regions_original.push(region_solution(a)?),
        "ConstraintSolution_FCAS_OCD" => {
            out.solution.fcas_ocd_constraints.push(FcasOcdConstraint {
                constraint_id: a.s("ConstraintID")?,
                intervention: a.b("Intervention")?,
                rhs: a.f("RHS")?,
                marginal_value: a.f("MarginalValue")?,
                violation_degree: a.f("ViolationDegree")?,
                other: a.0,
            })
        }
        "Violating_GenericConstraint" => out.ocd_violations.push(a.0),
        "InterconnectorSolution" => out.solution.interconnectors.push(InterconnectorSolution {
            interconnector_id: a.s("InterconnectorID")?,
            period_id: a.t("PeriodID")?,
            intervention: a.b("Intervention")?,
            flow: a.f("Flow")?,
            losses: a.f("Losses")?,
            inter_regional_loss_factor: a.of64("InterRegionalLossFactor")?,
            other: a.0,
        }),
        "TraderSolution" => out.solution.traders.push(TraderSolution {
            trader_id: a.s("TraderID")?,
            period_id: a.t("PeriodID")?,
            intervention: a.b("Intervention")?,
            energy_target: a.f("EnergyTarget")?,
            other: a.0,
        }),
        "ConstraintSolution" => out.solution.constraints.push(ConstraintSolution {
            constraint_id: a.s("ConstraintID")?,
            version: a.s("Version")?,
            period_id: a.t("PeriodID")?,
            intervention: a.b("Intervention")?,
            rhs: a.f("RHS")?,
            marginal_value: a.f("MarginalValue")?,
            deficit: a.f("Deficit")?,
            other: a.0,
        }),
        "PriceSetting" => out.price_setting.push(price_setting(a)?),
        "PriceSetting_Original" => out.price_setting_original.push(price_setting(a)?),
        _ => {
            if !a.0.is_empty() {
                out.unread.insert(name.to_string());
            }
        }
    }
    Ok(())
}

fn region_solution(mut a: A) -> Result<RegionSolution> {
    Ok(RegionSolution {
        region_id: a.s("RegionID")?,
        period_id: a.t("PeriodID")?,
        intervention: a.b("Intervention")?,
        energy_price: a.f("EnergyPrice")?,
        net_export: a.f("NetExport")?,
        other: a.0,
    })
}

fn price_setting(mut a: A) -> Result<PriceSetting> {
    let band_no = a.os("BandNo");
    Ok(PriceSetting {
        period_id: a.t("PeriodID")?,
        region_id: a.s("RegionID")?,
        market: a.s("Market")?,
        price: a.f("Price")?,
        unit: a.s("Unit")?,
        dispatched_market: a.s("DispatchedMarket")?,
        band_no,
        increase: a.f("Increase")?,
        rrn_band_price: a.f("RRNBandPrice")?,
        band_cost: a.f("BandCost")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_give_the_interval() {
        let n = parse_case_name("NEMSPDOutputs_2025071001800.loaded").unwrap();
        assert_eq!(n.day, NaiveDate::from_ymd_opt(2025, 7, 10).unwrap());
        assert_eq!(n.period, 18);
        assert_eq!(n.interval_end.to_string(), "2025-07-10 05:30:00");
        assert!(n.is_dispatch_run());
        let last = parse_case_name("NEMSPDOutputs_2025071028800.loaded").unwrap();
        assert_eq!(last.interval_end.to_string(), "2025-07-11 04:00:00");
        let ocd = parse_case_name("NEMSPDOutputs_20220613038_MANUALOCD_TOM00_OCD.loaded").unwrap();
        assert_eq!(ocd.interval_end.to_string(), "2022-06-13 07:10:00");
        assert_eq!(ocd.run, "_MANUALOCD_TOM00_OCD");
        assert!(!ocd.is_dispatch_run());
        assert!(parse_case_name("NemPriceSetter_20250710_xml.zip").is_none());
        assert!(parse_case_name("NEMSPDOutputs_2025071028900.loaded").is_none());
    }

    #[test]
    fn url_follows_the_archive_layout() {
        assert_eq!(
            day_zip_url(NaiveDate::from_ymd_opt(2020, 1, 31).unwrap()),
            "https://nemweb.com.au/Data_Archive/Wholesale_Electricity/NEMDE/2020/NEMDE_2020_01/NEMDE_Market_Data/NEMDE_Files/NemSpdOutputs_20200131_loaded.zip"
        );
    }

    #[test]
    fn rhs_terms_go_to_their_owner() {
        let xml = r#"<NEMSPDCaseFile><NemSpdInputs>
            <Case CaseID="1" CaseType="DS" Intervention="False" VoLL="1" MPF="-1"/>
            <GenericEquationCollection><GenericEquation EquationID="EQ"><RHSTermCollection>
              <RHSTerm TermID="1" Multiplier="2" SpdID="X" SpdType="A"/>
            </RHSTermCollection></GenericEquation></GenericEquationCollection>
            <GenericConstraintCollection><GenericConstraint ConstraintID="C" Version="v" Type="LE" RHS="5" ViolationPrice="9">
              <LHSFactorCollection><TraderFactor Factor="1" TradeType="ENOF" TraderID="U"/></LHSFactorCollection>
              <RHSTermCollection><RHSTerm TermID="1" Multiplier="1" SpdID="EQ" SpdType="X" Default="3"/></RHSTermCollection>
            </GenericConstraint></GenericConstraintCollection>
            <Surprise Attr="1"/>
          </NemSpdInputs></NEMSPDCaseFile>"#;
        let c = read_case(xml.as_bytes()).unwrap();
        assert_eq!(c.equations[0].rhs_terms[0].multiplier, 2.0);
        assert_eq!(c.constraints[0].rhs_terms[0].default, Some(3.0));
        assert_eq!(c.constraints[0].trader_factors[0].trader_id, "U");
        assert!(c.unread.contains("Surprise"));
    }

    #[test]
    fn rejects_other_xml() {
        assert!(read_case(b"<Other/>").is_err());
    }
}
