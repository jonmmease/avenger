use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum WeekStart {
    #[default]
    Sunday,
    Monday,
    Tuesday,
    Wednesday,
    Thursday,
    Friday,
    Saturday,
}

impl WeekStart {
    pub fn anchor_timestamp(&self) -> &'static str {
        match self {
            WeekStart::Sunday => "2012-01-01T00:00:00Z",
            WeekStart::Monday => "2024-01-01T00:00:00Z",
            WeekStart::Tuesday => "2008-01-01T00:00:00Z",
            WeekStart::Wednesday => "2020-01-01T00:00:00Z",
            WeekStart::Thursday => "2004-01-01T00:00:00Z",
            WeekStart::Friday => "2016-01-01T00:00:00Z",
            WeekStart::Saturday => "2000-01-01T00:00:00Z",
        }
    }

    pub fn anchor_year(&self) -> i32 {
        match self {
            WeekStart::Sunday => 2012,
            WeekStart::Monday => 2024,
            WeekStart::Tuesday => 2008,
            WeekStart::Wednesday => 2020,
            WeekStart::Thursday => 2004,
            WeekStart::Friday => 2016,
            WeekStart::Saturday => 2000,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimeContext {
    pub timezone: Option<String>,
    pub week_start: Option<WeekStart>,
}

impl TimeContext {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn timezone(mut self, timezone: impl Into<String>) -> Self {
        self.timezone = Some(timezone.into());
        self
    }

    pub fn week_start(mut self, week_start: WeekStart) -> Self {
        self.week_start = Some(week_start);
        self
    }

    pub fn resolved_week_start(&self) -> WeekStart {
        self.week_start.unwrap_or_default()
    }

    pub fn resolved_timezone(&self) -> &str {
        self.timezone.as_deref().unwrap_or("UTC")
    }

    pub fn resolved_with_parent(&self, parent: &TimeContext) -> TimeContext {
        TimeContext {
            timezone: self.timezone.clone().or_else(|| parent.timezone.clone()),
            week_start: self.week_start.or(parent.week_start),
        }
    }
}
