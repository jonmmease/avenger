-- The share of each region in the revenue of each month, from the offset `normalize`.
FROM samples.sales
|> AGGREGATE mark_bar(
     x => timeunit(month, unit => 'month'),
     y => y(stack(sum(revenue), offset => 'normalize'), title => 'Share of revenue'),
     color => region,
   )
