-- The datasets of the Vega examples, which the other data scripts and the charts read.
CREATE SCHEMA vega_datasets OPTIONS (description = 'The datasets of the Vega examples');

CREATE EXTERNAL TABLE vega_datasets.cars
OPTIONS (format = 'JSON', uris = ['data/files/vega_datasets/cars.json']);

LOAD DATA OVERWRITE vega_datasets.seattle_weather
FROM FILES (format = 'CSV', uris = ['data/files/vega_datasets/seattle-weather.csv']);
