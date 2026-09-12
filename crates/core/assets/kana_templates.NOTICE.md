# Kana template provenance

`kana_templates.json` is a deterministic collection of trajectory data from
the following sources. The application code around it is not derived from
either project.

## ctegaki-lib hiragana templates

- Source: <https://github.com/asdfjkl/ctegaki-lib>
- Pinned commit: `8a7b619ea77b50e427f2e9921304866230caf963`
- Files: the 46 XML documents under `hiragana/`
- Use here: their point trajectories are serialized as the hiragana half of
  `kana_templates.json`.
- License: 2-clause BSD

Copyright (c) 2014, Dominik Klein
All rights reserved.

Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions are met:

1. Redistributions of source code must retain the above copyright notice,
   this list of conditions and the following disclaimer.
2. Redistributions in binary form must reproduce the above copyright notice,
   this list of conditions and the following disclaimer in the documentation
   and/or other materials provided with the distribution.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS"
AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE
IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT OWNER OR CONTRIBUTORS BE LIABLE FOR
ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES
(INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES;
LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON
ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
(INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS
SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

## AnimCJK katakana templates

- Source: <https://github.com/parsimonhi/animCJK>
- Pinned commit: `ec5e17cca76c87587790bcbce5ea0b4d4fb753d6`
- File: `graphicsJaKana.txt`
- Use here: the `medians` trajectories of the 46 basic katakana are converted
  from font coordinates to screen coordinates (`x, 900 - y`) and serialized
  as the katakana half of `kana_templates.json`. The independent `フ` regression
  fixture in `tools/fixtures/kana-fu.json` and the tests uses the same pinned
  source's SVG centerline.
- License: GNU Lesser General Public License, version 3 or later. A copy of
  version 3 is in [`licenses/LGPL-3.0.txt`](licenses/LGPL-3.0.txt).

AnimCJK project - Copyright 2016-2026 - FM&SH

## Kuzushiji-49 hiragana variants

- Dataset: [Kuzushiji-49](https://codh.rois.ac.jp/kmnist/), published by the
  ROIS-DS Center for Open Data in the Humanities
- Source repository: <https://github.com/rois-codh/kmnist>
- Pinned source commit: `eb78dc13345cb00a9ced461e9be277f845534dff`
- Training-image SHA-256:
  `1c42adc463ed8efe598cf002f6ecafbca6d8c38f0551f7a35e0b23755fca1c8d`
- Training-label SHA-256:
  `fbfef4750ce9aa70b6072f0bca7daa9e80e2d79b379d5ae923265a63396260e4`
- Class-map SHA-256:
  `95f5a2cfbfba1721f566059cf77c22808898b63044a5f9a08de9d8a2950eed84`
- Use here: one representative training image for each of the 46 basic
  hiragana is selected from a deterministic 256-image pool by pixel-centroid
  medoid distance, then thresholded, thinned and traced into an order-free
  skeleton. The 24 samples per character reserved for development evaluation
  are excluded from selection. No official test image is embedded.
- Modifications: deterministic subset selection, Otsu thresholding,
  Zhang-Suen thinning, graph tracing, collinear simplification and JSON
  serialization as an additional trajectory variant.
- License: [Creative Commons Attribution-ShareAlike 4.0 International][cc].
  The K49-derived variants and modifications described above remain available
  under CC BY-SA 4.0. No endorsement by the dataset authors is implied.

Requested attribution: Tarin Clanuwat, Mikel Bober-Irizar, Asanobu Kitamoto,
Alex Lamb, Kazuaki Yamamoto and David Ha, “Deep Learning for Classical
Japanese Literature”, arXiv:1812.01718 (2018).

The generators record all pinned commits, hashes, selection parameters and
selected K49 record indices in the generated JSON. First regenerate the
canonical asset with `tools/build-kana-templates.py`, then add the K49 variants
with `tools/build-kana-k49-variants.py`; do not edit the serialized asset by
hand.

When only the canonical katakana importer changes, `build-kana-templates.py
--variants-from <previous-asset>` can retain the already generated K49 variants
and their full provenance. It verifies that canonical hiragana and its source
commit are unchanged before copying them; this does not reselect any prototype.

[cc]: https://creativecommons.org/licenses/by-sa/4.0/
