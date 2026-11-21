-- The range of temperatures of each month, as rules from the lowest to the highest.
FROM samples.seattle_weather
|> AGGREGATE mark_rule(
     x => x(timeunit(date, unit => 'month').start, title => 'Month'),
     y => y(min(temp_min), title => 'Temperature (°C)'),
     y2 => max(temp_max),
     stroke_width => 4,
   )
