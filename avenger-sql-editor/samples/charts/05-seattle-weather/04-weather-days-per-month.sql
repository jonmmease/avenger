-- The days of each kind of weather in each month.
FROM samples.seattle_weather
|> AGGREGATE mark_bar(x => timeunit(date, unit => 'month'), y => stack(count(*)), color => weather)
