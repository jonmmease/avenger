CREATE TEMP TABLE penguins AS (
  FROM penguins_csv
  |> SELECT species, island,
       SAFE_CAST(bill_length_mm AS FLOAT64) AS bill_length_mm,
       SAFE_CAST(bill_depth_mm AS FLOAT64) AS bill_depth_mm,
       SAFE_CAST(flipper_length_mm AS FLOAT64) AS flipper_length_mm,
       SAFE_CAST(body_mass_g AS FLOAT64) AS body_mass_g,
       NULLIF(sex, '') AS sex
);
CREATE TEMP TABLE sales AS (
  FROM sales_csv
  |> SELECT region, product, CAST(month AS DATE) AS month,
       SAFE_CAST(units AS INT64) AS units, CAST(price AS FLOAT64) AS price
);
CREATE TEMP TABLE events AS (
  FROM events_csv
  |> SELECT CAST(id AS INT64) AS id, CAST(CONCAT(ts, '+00') AS TIMESTAMP) AS ts, kind,
       IF(tags = '', [], SPLIT(tags, ';')) AS tags, SAFE_CAST(score AS FLOAT64) AS score
);
