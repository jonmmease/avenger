-- A table in the schema of another data script, which reads no table of it.
LOAD DATA OVERWRITE vega_datasets.regions
FROM FILES (format = 'CSV', uris = ['data/files/regions.csv']);
