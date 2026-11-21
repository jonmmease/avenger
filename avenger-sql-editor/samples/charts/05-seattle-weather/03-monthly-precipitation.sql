-- The precipitation of each unit of time, such as each month, as bars that span their units, from
-- `timeunit` with the unit of the variable `unit`.
DECLARE unit STRING DEFAULT 'month';
FROM samples.seattle_weather
|> AGGREGATE mark_bar(x => timeunit(date, unit => unit), y => sum(precipitation))
