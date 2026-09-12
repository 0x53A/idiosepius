# Kana vision model

`kana_vision.bin` contains learned parameters for the experimental kana CNN,
not copies of training images. Its format is documented in `kana_vision.rs`;
`kana_vision_report.json` records source and model checksums, selection,
comparison experiments, and limitations. `kana_vision_golden.json` is a
synthetic arithmetic fixture containing no ETL pixels.

Training data attribution:

**ETL Character Database / Electrotechnical Laboratory, Japanese Technical
Committee for Optical Character Recognition, ETL Character Database,
1973–1984.**

Source: ETL4, ETL5, and ETL7, obtained from AIST under the terms at
<https://etlcdb.db.aist.go.jp/download2/>. The raw data, derived images,
per-record metadata, and scan-derived test fixtures remain in ignored local
training storage and are not redistributed with the application.

The application's code license does not replace the ETL data terms.
See `KANA-VISION.md` at the repository root for usage and evaluation limits.
