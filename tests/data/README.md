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
