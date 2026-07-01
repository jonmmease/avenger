# NYC Taxi 2015 Test Data

This directory contains a small NYC taxi fixture for rasterization and
WebMercator-style chart baselines.

## File

- `nyc_taxi.csv`

The CSV has 50,000 data rows plus a header row. It is an unchanged copy of the
Datashader example data stub listed below.

SHA-256:

```text
e678ea3b786b9f4585d38111b6a139ed46c9237cb547d3b05779cafef23d9ce0
```

## Source

Upstream repository:

```text
https://github.com/holoviz/datashader
```

Pinned source file:

```text
https://raw.githubusercontent.com/holoviz/datashader/b7ac59c5ad20412edbf9b12ee9dc6ff62d563f01/examples/data/.data_stubs/nyc_taxi.csv
```

The HoloViz NYC Taxi example describes this dataset as part of the
well-studied NYC Taxi trip database, using pickup and dropoff locations from
January 2015.

## Useful Columns

- `pickup_x`, `pickup_y`: pickup coordinates in WebMercator meters.
- `dropoff_x`, `dropoff_y`: dropoff coordinates in WebMercator meters.
- `passenger_count`, `trip_distance`, `fare_amount`, `tip_amount`,
  `total_amount`: useful values for raster aggregation baselines.
- `payment_type`: useful grouping key for partitioned raster/facet baselines.

## License Notice

Datashader is distributed under a BSD license:

Copyright (c) 2015, Continuum Analytics, Inc. and contributors

All rights reserved.

Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions are met:

Redistributions of source code must retain the above copyright notice, this
list of conditions and the following disclaimer.

Redistributions in binary form must reproduce the above copyright notice, this
list of conditions and the following disclaimer in the documentation and/or
other materials provided with the distribution.

Neither the name of Continuum Analytics nor the names of any contributors may be
used to endorse or promote products derived from this software without specific
prior written permission.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND
ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE IMPLIED
WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT OWNER OR CONTRIBUTORS BE LIABLE FOR
ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES
(INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES;
LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON
ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
(INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS
SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
