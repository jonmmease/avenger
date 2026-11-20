-- Sepal width against sepal length, by species, from the chart's own table of the iris file.
--
-- `CREATE TEMP EXTERNAL TABLE` before the query names a table of the chart alone, which reads
-- its file at each run. No other chart and no data script sees it.
CREATE TEMP EXTERNAL TABLE iris
OPTIONS (format = 'CSV', uris = ['data/files/iris.csv']);

FROM iris
|> AGGREGATE mark_symbol(x => `SepalLengthCm`, y => `SepalWidthCm`, color => `Species`)
