-- The number of earthquakes in each year, stacked by depth, where shallow earthquakes are less than
-- 70 km deep.
FROM samples.earthquakes
|> AGGREGATE mark_bar(
     x => x(timeunit(time, unit => 'year'), title => 'Year'),
     y => y(stack(count(*)), title => 'Earthquakes'),
     color => color(if(depth_km < 70, 'Shallow', 'Deep'), title => 'Depth'),
   )
