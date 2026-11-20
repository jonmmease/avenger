-- Sepals and petals for each species of iris.
FROM samples.iris
|> AGGREGATE
     mark_symbol(x => `SepalLengthCm`, y => `SepalWidthCm`) AS sepals,
     mark_symbol(x => `PetalLengthCm`, y => `PetalWidthCm`) AS petals,
   GROUP BY `Species`
|> ORDER BY `Species`
