use crate::error::AvengerScaleError;
use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use chrono_tz::Tz;
use numfmt::Formatter;
use std::fmt::Debug;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DefaultFormatter {
    pub format_str: Option<String>,
    pub local_tz: Option<Tz>,
}

pub trait NumberFormatter: Debug + Send + Sync + 'static {
    fn format(&self, value: &[f32]) -> Vec<String>;
}

pub trait DateFormatter: Debug + Send + Sync + 'static {
    fn format(&self, value: &[NaiveDate]) -> Vec<String>;
}

pub trait TimestampFormatter: Debug + Send + Sync + 'static {
    fn format(&self, value: &[NaiveDateTime]) -> Vec<String>;
}

pub trait TimestamptzFormatter: Debug + Send + Sync + 'static {
    fn format(&self, value: &[DateTime<Utc>]) -> Vec<String>;
}

impl NumberFormatter for DefaultFormatter {
    fn format(&self, value: &[f32]) -> Vec<String> {
        if let Some(format_str) = &self.format_str {
            let mut f: Formatter = format_str.parse().unwrap(); // Fix panic
            value.iter().map(|&v| f.fmt2(v).to_string()).collect()
        } else {
            value.iter().map(|&v| v.to_string()).collect()
        }
    }
}

impl DateFormatter for DefaultFormatter {
    fn format(&self, value: &[NaiveDate]) -> Vec<String> {
        // If the format string is empty, just return the date as a string with default format
        if let Some(format_str) = &self.format_str {
            value
                .iter()
                .map(|v| v.format(format_str).to_string())
                .collect()
        } else {
            value.iter().map(|v| v.to_string()).collect()
        }
    }
}

impl TimestampFormatter for DefaultFormatter {
    fn format(&self, value: &[NaiveDateTime]) -> Vec<String> {
        // If the format string is empty, just return the date as a string with default format
        if let Some(format_str) = &self.format_str {
            value
                .iter()
                .map(|v| v.format(format_str).to_string())
                .collect()
        } else {
            value.iter().map(|v| v.to_string()).collect()
        }
    }
}

impl TimestamptzFormatter for DefaultFormatter {
    fn format(&self, value: &[DateTime<Utc>]) -> Vec<String> {
        if let Some(format_str) = &self.format_str {
            if let Some(local_tz) = &self.local_tz {
                value
                    .iter()
                    .map(|v| v.with_timezone(local_tz).format(format_str).to_string())
                    .collect()
            } else {
                value
                    .iter()
                    .map(|v| v.format(format_str).to_string())
                    .collect()
            }
        } else {
            // If the format string is empty, just return the date as a string with default format
            if let Some(local_tz) = &self.local_tz {
                value
                    .iter()
                    .map(|v| v.with_timezone(local_tz).to_string())
                    .collect()
            } else {
                value.iter().map(|v| v.to_string()).collect()
            }
        }
    }
}

#[derive(Debug)]
pub struct Formatters {
    pub number: Box<dyn NumberFormatter>,
    pub date: Box<dyn DateFormatter>,
    pub timestamp: Box<dyn TimestampFormatter>,
    pub timestamptz: Box<dyn TimestamptzFormatter>,
}

impl Default for Formatters {
    fn default() -> Self {
        Self {
            number: Box::new(DefaultFormatter::default()),
            date: Box::new(DefaultFormatter::default()),
            timestamp: Box::new(DefaultFormatter::default()),
            timestamptz: Box::new(DefaultFormatter::default()),
        }
    }
}
