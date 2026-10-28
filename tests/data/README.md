# Test data

Real AEMO files, downloaded from NEMWeb on 17 October 2026.

- `PUBLIC_DISPATCHIS_202610171205_0000000543344246.zip`: the DispatchIS report
  for the interval ending 12:05 market time on 17 October 2026, unmodified,
  from https://nemweb.com.au/Reports/Current/DispatchIS_Reports/.
- `NEXT_DAY_OFFER_ENERGY_SPARSE_20261016_ADPBA1_ARWF1.zip`: an extract of
  `PUBLIC_NEXT_DAY_OFFER_ENERGY_SPARSE_20261016_0000000543293471.zip` from
  https://nemweb.com.au/Reports/Current/Next_Day_Offer_Energy_SPARSE/. It keeps
  the C and I rows and every D row for DUIDs ADPBA1 and ARWF1, unchanged and in
  file order; all other units are removed to keep the file small.

Source data © AEMO, used under AEMO's copyright permissions
(https://www.aemo.com.au/privacy-and-legal-notices/copyright-permissions).

- `NEMSPDOutputs_2025071001800_extract.zip` and `NEMSPDOutputs_2020013101500_extract.zip`:
  extracts of the NEMDE case files for the intervals ending 05:30 on 10 Jul 2025
  and 05:15 on 31 Jan 2020, from `NemSpdOutputs_<day>_loaded.zip` in the NEMDE
  archive (https://nemweb.com.au/Data_Archive/Wholesale_Electricity/NEMDE/),
  downloaded 28 October 2026. Every kept element is unchanged. The extracts keep
  the case, all regions and interconnectors, the NSW1 energy price setters' units
  (plus ADPBA1 in 2025), the constraints naming them, and a few SCADA values,
  equations and entered values; all other units and constraints are removed.
