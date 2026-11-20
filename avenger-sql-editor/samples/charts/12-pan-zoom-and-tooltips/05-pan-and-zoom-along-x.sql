-- The highest temperature of each day, with pan and zoom along x only, and a tooltip of the day
-- nearest the pointer along x.
FROM samples.seattle_weather
|> AGGREGATE mark_line(
     x => date,
     y => temp_max,
     tools => [navigate(channels => ['x']), tooltip(nearest => TRUE, metric => 'along_x')],
   )
