// use std::sync::Arc;

// use arrow::datatypes::{DataType, Float32Type};
// use datafusion::execution::context::SessionContext;
// use datafusion::functions_aggregate::array_agg::array_agg;
// use datafusion::functions_aggregate::min_max::{max, min};
// use datafusion::functions_nested::make_array::make_array;
// use datafusion::functions_nested::sort::array_sort;
// use datafusion::logical_expr::{col, Expr, ExprSchemable, SortExpr, Subquery};
// use datafusion::prelude::*;
// use datafusion::scalar::ScalarValue;

// use crate::error::AvengerChartError;
// use crate::utils::ExprHelpers;

// // pub enum Domain {
// //     Values(Vec<Expr>),
// //     Range(Expr, Expr),
// //     Dataframe(DataFrame),
// // }

// #[derive(Debug, Clone)]
// pub struct Scale {
//     pub name: String,
//     pub
//     pub domain: Option<Expr>,
//     pub range: Option<Expr>,
// }

// impl Scale {
//     pub fn new(name: &str) -> Self {
//         Self {
//             name: name.to_string(),
//             domain: None,
//             range: None,
//         }
//     }
// }

// #[cfg(test)]
// mod tests {
//     use std::sync::Arc;

//     use datafusion::{
//         functions_aggregate::min_max::{max, min},
//         logical_expr::Subquery,
//         prelude::{ident, lit},
//         scalar::ScalarValue,
//     };

//     use crate::utils::ExprHelpers;

//     use super::*;

//     #[tokio::test]
//     async fn test_scale() -> Result<(), AvengerChartError> {
//         let instance: MyStruct = MyStruct::new().with_name("John");

//         let ctx = SessionContext::new();
//         let df = ctx
//             .read_csv(
//                 &format!(
//                     "{}/../examples/iris/data/Iris.csv",
//                     env!("CARGO_MANIFEST_DIR")
//                 ),
//                 CsvReadOptions::default(),
//             )
//             .await?;

//         df.clone().show().await?;

//         // Build scale
//         let scale = Scale {
//             name: "SepalLengthCm".to_string(),
//             domain: df
//                 .clone()
//                 .select_columns(&[
//                     "SepalLengthCm",
//                     "SepalWidthCm",
//                     "PetalLengthCm",
//                     "PetalWidthCm",
//                 ])?
//                 .span()?,
//         };

//         let uniques = df.select_columns(&["Species"])?.uniques(Some(true))?;
//         let uniques_val = uniques.eval_to_scalar(&ctx).await?;
//         println!("{:?}", uniques_val);

//         // println!("{:?}", uniques);
//         // uniques.clone().show().await?;

//         // let subquery = Subquery {
//         //     subquery: Arc::new(uniques.logical_plan().clone()),
//         //     outer_ref_columns: vec![],
//         // };
//         // let expr = Expr::ScalarSubquery(subquery);
//         // println!("{:?}", expr);
//         // println!("{:?}", expr.eval_to_scalar(&ctx).await?);

//         // let uniques_val = uniques.eval_to_scalar(&ctx).await?;
//         // let uniques_val = uniques.eval_to_scalar(&ctx).await?;
//         // println!("{:?}", uniques_val);

//         // let domain = df
//         //     .select_columns(&[
//         //         "SepalLengthCm",
//         //         "SepalWidthCm",
//         //         "PetalLengthCm",
//         //         "PetalWidthCm",
//         //     ])?
//         //     .span()?;

//         // println!("{:?}", domain);
//         // println!("{:?}", domain.eval_to_scalar(&ctx).await?);
//         // let domain = df.span(ident("SepalLengthCm"))?;
//         // let domain_val = domain.eval_to_scalar(&ctx).await?;
//         // println!("{:?}", domain_val);

//         // let max_sepal_length = df.scalar_aggregate(max(ident("SepalLengthCm")))?;
//         // let expr2 = max_sepal_length.add(lit(100.0));
//         // println!("{:?}", expr2);
//         // let expr2_val = expr2.eval_to_scalar(&ctx).await?;
//         // println!("expr2: {:?}", expr2_val);

//         // let df2 = df
//         //     .clone()
//         //     .select_columns(&["SepalLengthCm"])?
//         //     .union(df.clone().select_columns(&["SepalWidthCm"])?)?;
//         // df2.clone().show().await?;

//         // let combined_span = df2
//         //     .span(ident("SepalLengthCm"))?
//         //     .eval_to_scalar(&ctx)
//         //     .await?;
//         // println!("{:?}", combined_span);

//         Ok(())
//         // let df2 = df
//         // let df2 = df
//         //     .select_columns(&["SepalLengthCm", "SepalWidthCm"])
//         //     ?;
//         // df2.show().await?;

//         // println!("{:?}", df.schema());

//         // // Compute domain from column
//         // let df = df
//         //     .aggregate(
//         //         vec![],
//         //         vec![
//         //             min(ident("SepalLengthCm")).alias("min_a"),
//         //             max(ident("SepalLengthCm")).alias("max_a"),
//         //         ],
//         //     )
//         //     ?
//         //     .select(vec![
//         //         make_array(vec![ident("min_a"), ident("max_a")]).alias("domain")
//         //     ])
//         //     ?;

//         // df.clone().show().await?;

//         // let plan = df.logical_plan();
//         // println!("{:?}", plan);

//         // let subquery = Subquery {
//         //     subquery: Arc::new(plan.clone()),
//         //     outer_ref_columns: vec![],
//         // };
//         // let expr = Expr::ScalarSubquery(subquery);
//         // println!("{:?}", expr.eval_to_scalar().await?);

//         // let res = df.collect().await?;
//         // let row0 = res.get(0)?;
//         // let domain = row0.column_by_name("domain")?;
//         // let range = row0.column_by_name("range")?;

//         // println!("{:?}", domain);
//         // println!("{:?}", range);

//         // let expr = make_array(vec![lit(0.1), lit(10.0)]);
//         // println!("{:?}", expr);
//         // // }

//         // let expr = make_array(vec![lit(0.1), lit(10.0)]);
//         // println!("{:?}", expr);
//     }
// }

// // #[derive(Debug, Clone)]
// // pub struct LazyScalar {
// //     df: DataFrame,
// // }

// // impl LazyScalar {
// //     pub fn new(df: DataFrame) -> Result<Self, AvengerChartError> {
// //         if df.schema().fields().len() != 1 {
// //             return Err(AvengerChartError::InternalError(
// //                 "LazyScalar requires a single column dataframe".to_string(),
// //             ));
// //         }

// //         Ok(Self { df })
// //     }

// //     pub fn data_type(&self) -> &DataType {
// //         self.df.schema().fields()[0].data_type()
// //     }

// //     pub async fn collect(self) -> Result<ScalarValue, AvengerChartError> {
// //         let res = self.df.collect().await?;
// //         if res.len() != 1 {
// //             return Err(AvengerChartError::InternalError(
// //                 "LazyScalar requires a single row dataframe".to_string(),
// //             ));
// //         }
// //         let batch0 = res.get(0)?;
// //         if batch0.num_rows() != 1 {
// //             return Err(AvengerChartError::InternalError(
// //                 "LazyScalar requires a single row dataframe".to_string(),
// //             ));
// //         }
// //         let col0 = batch0.column(0);

// //         Ok(ScalarValue::try_from_array(col0, 0)?)
// //     }
// // }

// // pub fn span(df: &[DataFrame]) -> Result<Expr, AvengerChartError> {
// //     let df = df.iter().map(|df| df.select_columns(&[col])).collect::<Result<Vec<_>>>()?;
// //     let df = df.iter().map(|df| df.span(ident(col))).collect::<Result<Vec<_>>>()?;
// //     Ok(df)
// // }

// pub trait DataFrameChartUtils {
//     fn span(&self) -> Result<Expr, AvengerChartError>;
//     fn uniques(&self, sort_ascending: Option<bool>) -> Result<Expr, AvengerChartError>;
//     fn scalar_aggregate(&self, expr: Expr) -> Result<Expr, AvengerChartError>;
// }

// impl DataFrameChartUtils for DataFrame {
//     fn span(&self) -> Result<Expr, AvengerChartError> {
//         // Collect single column DataFrames for all numeric columns
//         let mut union_dfs: Vec<DataFrame> = Vec::new();
//         let col_name = "span_col";
//         for field in self.schema().fields() {
//             if field.data_type().is_numeric() {
//                 union_dfs.push(
//                     self.clone()
//                         .select(vec![ident(field.name())
//                             .try_cast_to(&DataType::Float32, self.schema())?
//                             .alias(col_name)])?
//                         .clone(),
//                 );
//             }
//         }

//         if union_dfs.is_empty() {
//             return Err(AvengerChartError::InternalError(
//                 "No numeric columns found for span".to_string(),
//             ));
//         }

//         // Union all the DataFrames
//         let union_df = union_dfs.iter().fold(union_dfs[0].clone(), |acc, df| {
//             acc.union(df.clone()).unwrap()
//         });

//         // Compute domain from column
//         let df = union_df
//             .clone()
//             .aggregate(
//                 vec![],
//                 vec![
//                     min(ident(col_name)).alias("min_val"),
//                     max(ident(col_name)).alias("max_val"),
//                 ],
//             )?
//             .select(vec![
//                 make_array(vec![ident("min_val"), ident("max_val")]).alias("span")
//             ])?;

//         let subquery = Subquery {
//             subquery: Arc::new(df.logical_plan().clone()),
//             outer_ref_columns: vec![],
//         };
//         Ok(Expr::ScalarSubquery(subquery))
//     }

//     fn uniques(&self, sort_ascending: Option<bool>) -> Result<Expr, AvengerChartError> {
//         // Collect single column DataFrames for all columns. Let DataFusion try to unify the types
//         let mut union_dfs: Vec<DataFrame> = Vec::new();
//         let col_name = "vals";
//         for field in self.schema().fields() {
//             union_dfs.push(
//                 self.clone()
//                     .select(vec![ident(field.name()).alias(col_name)])?
//                     .clone(),
//             );
//         }

//         if union_dfs.is_empty() {
//             return Err(AvengerChartError::InternalError(
//                 "No columns found for uniques".to_string(),
//             ));
//         }

//         // Union all the DataFrames
//         let union_df = union_dfs.iter().fold(union_dfs[0].clone(), |acc, df| {
//             acc.union(df.clone()).unwrap()
//         });

//         let mut uniques_df = union_df.clone().distinct()?;

//         // Collect unique values in array
//         uniques_df =
//             uniques_df.aggregate(vec![], vec![array_agg(ident(col_name)).alias("vals_array")])?;

//         let subquery = Subquery {
//             subquery: Arc::new(uniques_df.logical_plan().clone()),
//             outer_ref_columns: vec![],
//         };

//         let expr = Expr::ScalarSubquery(subquery);

//         if let Some(sort_ascending) = sort_ascending {
//             Ok(array_sort(expr, lit("desc"), lit("NULLS LAST")))
//         } else {
//             Ok(expr)
//         }
//     }

//     fn scalar_aggregate(&self, expr: Expr) -> Result<Expr, AvengerChartError> {
//         let df = self
//             .clone()
//             .aggregate(vec![], vec![expr.alias("val")])?
//             .select(vec![ident("val")])?;
//         let subquery = Subquery {
//             subquery: Arc::new(df.logical_plan().clone()),
//             outer_ref_columns: vec![],
//         };
//         Ok(Expr::ScalarSubquery(subquery))
//     }
// }
