use arrow::array::{ArrayRef, Float32Array};
use avenger_scales::scales::linear::LinearScale;
use chrono::{DateTime, NaiveDate, Utc};
use std::sync::Arc;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== Formatting and Tick Generation Examples ===\n");

    // Example 1: Numeric Formatting
    println!("1. Number Formatting:");

    let scale = LinearScale::configured((0.0, 1000.0), (0.0, 500.0));

    let numbers = vec![0.0, 123.456, 1000.0, 10000.0, 0.00123];
    let number_array = Arc::new(Float32Array::from(numbers.clone())) as ArrayRef;

    let formatted_result = scale.scale_to_string(&number_array)?;
    let formatted_numbers = formatted_result.as_vec(numbers.len(), None);

    println!("Default number formatting:");
    for (num, formatted) in numbers.iter().zip(formatted_numbers.iter()) {
        println!("  {:.5} → '{}'", num, formatted);
    }

    // Format specific numbers with different precision
    let precision_numbers = vec![
        Some(std::f32::consts::PI),
        Some(std::f32::consts::E),
        None,
        Some(std::f32::consts::SQRT_2),
    ];
    let formatted_precision = scale.format_numbers(&precision_numbers);
    let precision_strings = formatted_precision.as_vec(precision_numbers.len(), None);

    println!("\nNumber formatting with Some/None values:");
    for (num, formatted) in precision_numbers.iter().zip(precision_strings.iter()) {
        match num {
            Some(n) => println!("  Some({:.5}) → '{}'", n, formatted),
            None => println!("  None → '{}'", formatted),
        }
    }

    // Example 2: Date Formatting
    println!("\n2. Date Formatting:");

    let dates = vec![
        Some(NaiveDate::from_ymd_opt(2024, 1, 15).unwrap()),
        Some(NaiveDate::from_ymd_opt(2024, 6, 30).unwrap()),
        None,
        Some(NaiveDate::from_ymd_opt(2024, 12, 25).unwrap()),
    ];

    let formatted_dates = scale.format_dates(&dates);
    let date_strings = formatted_dates.as_vec(dates.len(), None);

    println!("Date formatting:");
    for (date, formatted) in dates.iter().zip(date_strings.iter()) {
        match date {
            Some(d) => println!("  {} → '{}'", d, formatted),
            None => println!("  None → '{}'", formatted),
        }
    }

    // Example 3: Timestamp Formatting
    println!("\n3. Timestamp Formatting:");

    let timestamps = vec![
        DateTime::from_timestamp(1640995200, 0).map(|dt| dt.naive_utc()), // 2022-01-01 00:00:00
        DateTime::from_timestamp(1672531200, 0).map(|dt| dt.naive_utc()), // 2023-01-01 00:00:00
        None,
        DateTime::from_timestamp(1704067200, 0).map(|dt| dt.naive_utc()), // 2024-01-01 00:00:00
    ];

    let formatted_timestamps = scale.format_timestamps(&timestamps);
    let timestamp_strings = formatted_timestamps.as_vec(timestamps.len(), None);

    println!("Timestamp formatting:");
    for (ts, formatted) in timestamps.iter().zip(timestamp_strings.iter()) {
        match ts {
            Some(t) => println!("  {} → '{}'", t, formatted),
            None => println!("  None → '{}'", formatted),
        }
    }

    // Example 4: Timezone-aware Timestamp Formatting
    println!("\n4. Timezone-aware Timestamp Formatting:");

    let base_time = DateTime::from_timestamp(1640995200, 0)
        .unwrap()
        .with_timezone(&Utc);
    let timestamptz_values = vec![
        Some(base_time),
        Some(base_time + chrono::Duration::hours(6)),
        None,
        Some(base_time + chrono::Duration::days(365)),
    ];

    let formatted_timestamptz = scale.format_timestamptz(&timestamptz_values);
    let timestamptz_strings = formatted_timestamptz.as_vec(timestamptz_values.len(), None);

    println!("Timezone-aware timestamp formatting:");
    for (ts, formatted) in timestamptz_values.iter().zip(timestamptz_strings.iter()) {
        match ts {
            Some(t) => println!("  {} → '{}'", t, formatted),
            None => println!("  None → '{}'", formatted),
        }
    }

    // Example 5: Tick Generation
    println!("\n5. Tick Generation:");

    // Linear scale ticks
    let tick_scale = LinearScale::configured((0.0, 100.0), (0.0, 400.0));

    println!("Linear scale ticks (different counts):");
    for tick_count in [5.0, 10.0, 15.0] {
        let ticks = tick_scale.ticks(Some(tick_count))?;
        let tick_array = ticks.as_any().downcast_ref::<Float32Array>().unwrap();
        let tick_values: Vec<f32> = tick_array.values().to_vec();
        println!("  {} ticks: {:?}", tick_count, tick_values);
    }

    // Example 6: Scale with Custom Options
    println!("\n6. Scale with Custom Options:");

    let custom_scale = LinearScale::configured((0.0, 1000.0), (0.0, 500.0))
        .with_option("clamp", true)
        .with_option("round", true)
        .with_option("nice", true);

    let test_values = vec![-100.0, 500.0, 1200.0]; // Values outside domain
    let test_array = Arc::new(Float32Array::from(test_values.clone())) as ArrayRef;

    let clamped_result = custom_scale.scale_to_numeric(&test_array)?;
    let clamped_values = clamped_result.as_vec(test_values.len(), None);

    println!("Scale with clamping and rounding:");
    for (input, output) in test_values.iter().zip(clamped_values.iter()) {
        println!("  {:.1} → {:.1}", input, output);
    }

    // Show domain and range info
    let (domain_start, domain_end) = custom_scale.numeric_interval_domain()?;
    let (range_start, range_end) = custom_scale.numeric_interval_range()?;

    println!("\nScale configuration:");
    println!("  Domain: [{}, {}]", domain_start, domain_end);
    println!("  Range: [{}, {}]", range_start, range_end);
    println!("  Clamp: {}", custom_scale.option_boolean("clamp", false));
    println!("  Round: {}", custom_scale.option_boolean("round", false));
    println!("  Nice: {}", custom_scale.option_boolean("nice", false));

    Ok(())
}
