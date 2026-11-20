-- The revenue of each region, stacked by product, with a wider gap between the panels.
FROM samples.sales
|> AGGREGATE mark_area(x => month, y => stack(sum(revenue)), color => product) AS chart
   GROUP BY region
|> AGGREGATE facet_column(chart, region, gap => 24) AS chart
