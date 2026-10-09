use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::download::Category;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DailyStat {
    /// Local date "YYYY-MM-DD".
    pub day: String,
    #[ts(type = "number")]
    pub bytes: u64,
    pub files: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct MonthlyStat {
    /// "YYYY-MM".
    pub month: String,
    #[ts(type = "number")]
    pub bytes: u64,
    pub files: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CategoryStat {
    pub category: Category,
    pub files: u32,
    #[ts(type = "number")]
    pub bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct StatsSummary {
    /// Bytes transferred over the network, all time.
    #[ts(type = "number")]
    pub total_bytes: u64,
    pub total_completed: u32,
    pub total_failed: u32,
    #[ts(type = "number")]
    pub today_bytes: u64,
    #[ts(type = "number")]
    pub month_bytes: u64,
    /// Last 30 days, oldest first.
    pub daily: Vec<DailyStat>,
    /// Last 12 months, oldest first.
    pub monthly: Vec<MonthlyStat>,
    pub by_category: Vec<CategoryStat>,
    /// Highest average speed of a completed download, bytes/second.
    #[ts(type = "number")]
    pub best_avg_speed: u64,
}
