-- The range of temperatures of each month, with a line and points at the mean of the highest
-- temperatures.
FROM samples.seattle_weather
|> EXTEND date_trunc(date, MONTH) AS month
|> AGGREGATE layer(
     mark_rule(
       x => month,
       y => y(min(temp_min), title => 'Temperature (°C)'),
       y2 => max(temp_max),
       opacity => 0.3,
       stroke_width => 6,
     ),
     mark_line(x => month, y => y(avg(temp_max), title => 'Temperature (°C)')),
     mark_symbol(x => month, y => y(avg(temp_max), title => 'Temperature (°C)'), size => 40),
   )
