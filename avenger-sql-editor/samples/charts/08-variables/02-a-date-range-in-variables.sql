-- The highest temperature of each day in the six months from the start date, from two variables.
DECLARE start_date DATE DEFAULT DATE '2014-01-01';
DECLARE months INT64 DEFAULT 6;
FROM samples.seattle_weather
|> WHERE date >= start_date AND date < date_add(start_date, INTERVAL months MONTH)
|> AGGREGATE mark_line(x => date, y => y(temp_max, title => 'Highest temperature (°C)'))
