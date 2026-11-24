-- Summaries of the datasets, through a step that the charts do not see.
CREATE TEMP TABLE heavy AS (
    FROM vega_datasets.cars
    |> WHERE Weight_in_lbs > 3000
);

CREATE TABLE heavy_by_origin AS (
    FROM heavy
    |> AGGREGATE COUNT(*) AS cars GROUP BY Origin
);

CREATE VIEW wet_days AS (
    FROM vega_datasets.seattle_weather
    |> WHERE precipitation > 0
);
