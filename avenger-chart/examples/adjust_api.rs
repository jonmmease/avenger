//! Examples of using the adjust API with marks

use avenger_chart::adjust::{Adjust, AdjustFn, Dodge, Jitter, PlotDimensions, TransformContext};
use avenger_chart::error::AvengerChartError;
use datafusion::prelude::*;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = SessionContext::new();

    // Create sample data
    let df = ctx
        .sql(
            "SELECT 
        'A' as category,
        1.0 as value,
        10.0 as x,
        20.0 as y,
        'Group1' as group,
        5 as count,
        'Label A' as label
    ",
        )
        .await?;

    // Example 1: Using a built-in adjustment
    // Note: In a real implementation, marks would have an adjust method
    let _jitter_adjustment = Jitter::new()
        .x(10.0) // ±10 pixels horizontal jitter
        .seed(42);

    println!("Created jitter adjustment");

    // Example 2: Using a lambda with wrapper
    let nudge_adjustment = AdjustFn::new(|df, _context| {
        // Nudge all points up by 5 pixels
        Ok(df.with_column("y", col("y") - lit(5.0))?)
    });

    println!("Created nudge adjustment with wrapper");

    // Example 3: Lambda with viewport awareness
    let center_adjustment = AdjustFn::new(|df, context| {
        // Center points if they're in the left half
        let condition = col("x").lt(lit(context.width() / 2.0));
        let new_x = when(condition, col("x") + lit(context.width() / 4.0)).otherwise(col("x"))?;

        Ok(df.with_column("x", new_x)?)
    });

    println!("Created centering adjustment with wrapper");

    // Example 4: Lambda using bounding box information
    let _overlap_avoidance = AdjustFn::new(|df, _context| {
        // In a real implementation, this would avoid overlaps
        // For now, just demonstrate the closure works
        println!("Would avoid overlaps here");
        Ok(df)
    });

    // Example 5: Combining multiple adjustments
    let _dodge_adjustment = Dodge::new().by("group");
    let _jitter_y = Jitter::new().y(3.0);

    println!("Created dodge and jitter adjustments");

    // Example 6: Custom struct implementing Adjust trait
    struct ForceSimulation {
        iterations: usize,
        charge: f64,
    }

    impl Adjust for ForceSimulation {
        fn adjust(
            &self,
            mut df: DataFrame,
            context: &TransformContext,
        ) -> Result<DataFrame, AvengerChartError> {
            // Simplified force simulation
            for _ in 0..self.iterations {
                let center_x = context.width() / 2.0;
                let center_y = context.height() / 2.0;

                let dx = col("x") - lit(center_x);
                let dy = col("y") - lit(center_y);
                // Simplified: just use a fixed force
                let force = lit(self.charge * 0.1);
                let new_x = col("x") + dx.clone() * force.clone();
                let new_y = col("y") + dy.clone() * force;

                df = df.with_column("x", new_x)?.with_column("y", new_y)?;
            }

            Ok(df)
        }
    }

    let force_sim = ForceSimulation {
        iterations: 5,
        charge: -10.0,
    };

    // Test the adjustments
    let test_dims = PlotDimensions {
        width: 800.0,
        height: 600.0,
    };
    let test_context = TransformContext::new(test_dims, ctx.clone());

    // Test applying wrapped adjustments
    let _adjusted_df = nudge_adjustment.adjust(df.clone(), &test_context)?;
    println!("Applied nudge adjustment successfully");

    // Test the Adjust trait implementation for wrapped closures
    let _adjusted_df2 = center_adjustment.adjust(df.clone(), &test_context)?;
    println!("Applied center adjustment via trait");

    // Test the force simulation
    let _adjusted_df3 = force_sim.adjust(df.clone(), &test_context)?;
    println!("Applied force simulation");

    Ok(())
}
