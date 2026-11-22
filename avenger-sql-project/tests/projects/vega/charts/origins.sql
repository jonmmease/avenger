-- The heavy cars of each region.
FROM heavy_by_origin
|> JOIN vega_datasets.regions USING (Origin)
|> AGGREGATE mark_bar(x => region, y => cars) AS chart
