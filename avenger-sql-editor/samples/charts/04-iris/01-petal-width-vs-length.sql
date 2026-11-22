-- Petal width against petal length, by species.
FROM samples.iris
|> AGGREGATE mark_symbol(x => `PetalLengthCm`, y => `PetalWidthCm`, color => `Species`)
