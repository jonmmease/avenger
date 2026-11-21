-- The revenue of each region, by month and stacked by product, and its units, by product and
-- stacked by channel, from two mark calls of one `AGGREGATE`, with the total revenue of each
-- region.
FROM samples.sales
|> AGGREGATE
     mark_area(x => month, y => stack(sum(revenue)), color => product) AS revenue,
     mark_bar(x => product, y => stack(sum(units)), color => channel) AS units,
     sum(revenue) AS total_revenue,
   GROUP BY region
|> ORDER BY total_revenue DESC
