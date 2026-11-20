-- The units of each product, stacked by channel.
FROM samples.sales
|> AGGREGATE mark_bar(x => product, y => y(stack(sum(units)), title => 'Units'), color => channel)
