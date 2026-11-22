-- The sample tables, which the charts of the sample project read. Each file is small and every
-- chart reads its table again, so `LOAD DATA` parses each file once, when the project loads, and
-- keeps its rows in memory. `data/files/README.md` gives the source and the terms of each file.
CREATE SCHEMA samples OPTIONS (description = 'The sample tables of the editor');

LOAD DATA OVERWRITE samples.penguins
OPTIONS (description = 'The Palmer Archipelago penguins')
FROM FILES (format = 'CSV', uris = ['data/files/penguins.csv']);

LOAD DATA OVERWRITE samples.cars
OPTIONS (description = 'The cars of the vega-datasets, by model year')
FROM FILES (format = 'CSV', uris = ['data/files/cars.csv']);

LOAD DATA OVERWRITE samples.movies
OPTIONS (description = 'Movies with their ratings, genre, and release year')
FROM FILES (format = 'CSV', uris = ['data/files/movies.csv']);

LOAD DATA OVERWRITE samples.iris
OPTIONS (description = 'The iris flowers of Fisher (1936)')
FROM FILES (format = 'CSV', uris = ['data/files/iris.csv']);

LOAD DATA OVERWRITE samples.seattle_weather
OPTIONS (description = 'The daily weather of Seattle from 2012 through 2015')
FROM FILES (format = 'CSV', uris = ['data/files/seattle_weather.csv']);

LOAD DATA OVERWRITE samples.sales
OPTIONS (description = 'Illustrative monthly revenue and units of 2025')
FROM FILES (format = 'CSV', uris = ['data/files/sales.csv']);

-- The geographic tables, from the Arrow IPC streams of avenger-guides-geo, which the link
-- `data/files/geo` reaches. A GeoArrow point of an earthquake becomes its `longitude` and `latitude`,
-- which `mark_geo_symbol` takes.
CREATE TEMP EXTERNAL TABLE earthquake_points
OPTIONS (format = 'ARROW', uris = ['data/files/geo/earthquakes.arrows']);

CREATE TABLE samples.earthquakes
OPTIONS (description = 'The earthquakes of magnitude 6 and larger from 2000 through 2024')
AS (
  FROM earthquake_points
  |> SELECT
       geometry[OFFSET(0)] AS longitude,
       geometry[OFFSET(1)] AS latitude,
       * EXCEPT (geometry)
);

LOAD DATA OVERWRITE samples.countries
OPTIONS (description = 'The outlines of the countries, as GeoArrow multipolygons')
FROM FILES (format = 'ARROW', uris = ['data/files/geo/countries.arrows']);

LOAD DATA OVERWRITE samples.plate_boundaries
OPTIONS (description = 'The boundaries of the tectonic plates, as WKB lines')
FROM FILES (format = 'ARROW', uris = ['data/files/geo/plate_boundaries.arrows']);
