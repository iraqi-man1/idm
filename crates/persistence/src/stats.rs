use chrono::{Datelike, Duration, Local, NaiveDate};
use rusqlite::params;
use velox_types::{Category, CategoryStat, DailyStat, MonthlyStat, StatsSummary};

use crate::{from_i64, to_i64, Database, DbResult};

fn today() -> NaiveDate {
    Local::now().date_naive()
}

impl Database {
    /// Add transferred bytes to today's statistics.
    pub fn add_transferred(&self, bytes: u64) -> DbResult<()> {
        if bytes == 0 {
            return Ok(());
        }
        let day = today().format("%Y-%m-%d").to_string();
        self.with(|c| {
            c.prepare_cached(
                "INSERT INTO stats_daily (day, bytes) VALUES (?1, ?2)
                 ON CONFLICT(day) DO UPDATE SET bytes = bytes + excluded.bytes",
            )?
            .execute(params![day, to_i64(bytes)])?;
            Ok(())
        })
    }

    /// Count a finished (or failed) file in today's statistics.
    pub fn add_finished(&self, success: bool) -> DbResult<()> {
        let day = today().format("%Y-%m-%d").to_string();
        let (files, failed) = if success { (1, 0) } else { (0, 1) };
        self.with(|c| {
            c.prepare_cached(
                "INSERT INTO stats_daily (day, files, failed) VALUES (?1, ?2, ?3)
                 ON CONFLICT(day) DO UPDATE SET files = files + excluded.files,
                                                failed = failed + excluded.failed",
            )?
            .execute(params![day, files, failed])?;
            Ok(())
        })
    }

    pub fn stats_summary(&self) -> DbResult<StatsSummary> {
        let now = today();
        let today_s = now.format("%Y-%m-%d").to_string();
        let month_prefix = now.format("%Y-%m").to_string();
        let start_30 = (now - Duration::days(29)).format("%Y-%m-%d").to_string();
        self.with(|c| {
            let (total_bytes, total_files, total_failed): (i64, i64, i64) = c.query_row(
                "SELECT COALESCE(SUM(bytes),0), COALESCE(SUM(files),0), COALESCE(SUM(failed),0)
                 FROM stats_daily",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )?;
            let today_bytes: i64 = c.query_row(
                "SELECT COALESCE(SUM(bytes),0) FROM stats_daily WHERE day = ?1",
                [&today_s],
                |r| r.get(0),
            )?;
            let month_bytes: i64 = c.query_row(
                "SELECT COALESCE(SUM(bytes),0) FROM stats_daily WHERE substr(day,1,7) = ?1",
                [&month_prefix],
                |r| r.get(0),
            )?;

            // Daily series for the last 30 days, zero-filled.
            let mut stmt = c.prepare_cached(
                "SELECT day, bytes, files FROM stats_daily WHERE day >= ?1 ORDER BY day",
            )?;
            let rows: Vec<(String, i64, i64)> = stmt
                .query_map([&start_30], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                .collect::<Result<_, _>>()?;
            let mut daily = Vec::with_capacity(30);
            for i in (0..30).rev() {
                let d = (now - Duration::days(i)).format("%Y-%m-%d").to_string();
                let (bytes, files) = rows
                    .iter()
                    .find(|(day, _, _)| *day == d)
                    .map(|(_, b, f)| (from_i64(*b), *f as u32))
                    .unwrap_or((0, 0));
                daily.push(DailyStat {
                    day: d,
                    bytes,
                    files,
                });
            }

            // Monthly series for the last 12 months, zero-filled.
            let mut stmt = c.prepare_cached(
                "SELECT substr(day,1,7) AS m, SUM(bytes), SUM(files) FROM stats_daily GROUP BY m",
            )?;
            let mrows: Vec<(String, i64, i64)> = stmt
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                .collect::<Result<_, _>>()?;
            let mut monthly = Vec::with_capacity(12);
            let (mut y, mut m) = (now.year(), now.month() as i32);
            let mut months = Vec::new();
            for _ in 0..12 {
                months.push(format!("{y:04}-{m:02}"));
                m -= 1;
                if m == 0 {
                    m = 12;
                    y -= 1;
                }
            }
            months.reverse();
            for month in months {
                let (bytes, files) = mrows
                    .iter()
                    .find(|(mm, _, _)| *mm == month)
                    .map(|(_, b, f)| (from_i64(*b), *f as u32))
                    .unwrap_or((0, 0));
                monthly.push(MonthlyStat {
                    month,
                    bytes,
                    files,
                });
            }

            let mut stmt = c.prepare_cached(
                "SELECT category, COUNT(*), COALESCE(SUM(total_size),0) FROM downloads
                 WHERE status = 'completed' GROUP BY category",
            )?;
            let by_category = stmt
                .query_map([], |r| {
                    Ok(CategoryStat {
                        category: Category::parse(&r.get::<_, String>(0)?),
                        files: r.get::<_, i64>(1)? as u32,
                        bytes: from_i64(r.get(2)?),
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;

            let best_avg_speed: i64 = c.query_row(
                "SELECT COALESCE(MAX(total_size * 1000 / elapsed_ms), 0) FROM downloads
                 WHERE status = 'completed' AND elapsed_ms > 1000 AND total_size IS NOT NULL",
                [],
                |r| r.get(0),
            )?;

            Ok(StatsSummary {
                total_bytes: from_i64(total_bytes),
                total_completed: total_files as u32,
                total_failed: total_failed as u32,
                today_bytes: from_i64(today_bytes),
                month_bytes: from_i64(month_bytes),
                daily,
                monthly,
                by_category,
                best_avg_speed: from_i64(best_avg_speed),
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stats_accumulate() {
        let db = Database::open_in_memory().unwrap();
        db.add_transferred(1000).unwrap();
        db.add_transferred(500).unwrap();
        db.add_finished(true).unwrap();
        db.add_finished(false).unwrap();
        let s = db.stats_summary().unwrap();
        assert_eq!(s.total_bytes, 1500);
        assert_eq!(s.today_bytes, 1500);
        assert_eq!(s.month_bytes, 1500);
        assert_eq!(s.total_completed, 1);
        assert_eq!(s.total_failed, 1);
        assert_eq!(s.daily.len(), 30);
        assert_eq!(s.daily.last().unwrap().bytes, 1500);
        assert_eq!(s.monthly.len(), 12);
        assert_eq!(s.monthly.last().unwrap().bytes, 1500);
    }
}
