-- The same days as a stacked area. Snow falls in 7 of the 48 months, and the stacked area imputes 0
-- days of snow in the other months.
FROM samples.seattle_weather
|> AGGREGATE mark_area(x => timeunit(date, unit => 'month'), y => stack(count(*)), color => weather)
