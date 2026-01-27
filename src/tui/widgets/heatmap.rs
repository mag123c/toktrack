//! 52-week heatmap widget

use chrono::NaiveDate;
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style},
    widgets::Widget,
};

/// Heatmap intensity level based on percentiles
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeatmapIntensity {
    /// No usage (0 tokens)
    None,
    /// Low usage (1-25th percentile)
    Low,
    /// Medium usage (25-50th percentile)
    Medium,
    /// High usage (50-75th percentile)
    High,
    /// Max usage (75-100th percentile)
    Max,
}

impl HeatmapIntensity {
    /// Convert intensity to display character
    pub fn to_char(self) -> char {
        match self {
            Self::None => ' ',
            Self::Low => '░',
            Self::Medium => '▒',
            Self::High => '▓',
            Self::Max => '█',
        }
    }

    /// Get color for this intensity
    pub fn color(self) -> Color {
        match self {
            Self::None => Color::DarkGray,
            Self::Low => Color::Green,
            Self::Medium => Color::Yellow,
            Self::High => Color::Rgb(255, 165, 0), // Orange
            Self::Max => Color::Red,
        }
    }
}

/// Percentile thresholds for intensity mapping
#[derive(Debug, Clone, Copy)]
pub struct Percentiles {
    pub p25: u64,
    pub p50: u64,
    pub p75: u64,
}

impl Percentiles {
    /// Map a token count to intensity level
    pub fn intensity(self, tokens: u64) -> HeatmapIntensity {
        if tokens == 0 {
            HeatmapIntensity::None
        } else if tokens <= self.p25 {
            HeatmapIntensity::Low
        } else if tokens <= self.p50 {
            HeatmapIntensity::Medium
        } else if tokens <= self.p75 {
            HeatmapIntensity::High
        } else {
            HeatmapIntensity::Max
        }
    }
}

/// Calculate percentiles from a list of token counts (excluding zeros)
pub fn calculate_percentiles(values: &[u64]) -> Option<Percentiles> {
    let mut non_zero: Vec<u64> = values.iter().copied().filter(|&v| v > 0).collect();
    if non_zero.is_empty() {
        return None;
    }

    non_zero.sort_unstable();
    let len = non_zero.len();

    let p25_idx = (len as f64 * 0.25).ceil() as usize - 1;
    let p50_idx = (len as f64 * 0.50).ceil() as usize - 1;
    let p75_idx = (len as f64 * 0.75).ceil() as usize - 1;

    Some(Percentiles {
        p25: non_zero[p25_idx.min(len - 1)],
        p50: non_zero[p50_idx.min(len - 1)],
        p75: non_zero[p75_idx.min(len - 1)],
    })
}

/// A single cell in the heatmap grid
#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
pub struct HeatmapCell {
    pub date: NaiveDate,
    pub tokens: u64,
    pub intensity: HeatmapIntensity,
}

/// Build a 7x52 grid of heatmap cells (rows = weekdays, cols = weeks)
/// Fills from today going back 52 weeks
pub fn build_grid(
    daily_tokens: &[(NaiveDate, u64)],
    today: NaiveDate,
) -> Vec<Vec<Option<HeatmapCell>>> {
    use chrono::{Datelike, Duration};

    // Create a lookup map
    let token_map: std::collections::HashMap<NaiveDate, u64> =
        daily_tokens.iter().copied().collect();

    // Calculate percentiles for intensity mapping
    let all_values: Vec<u64> = daily_tokens.iter().map(|(_, t)| *t).collect();
    let percentiles = calculate_percentiles(&all_values);

    // Find the start of the current week (Monday)
    let days_since_monday = today.weekday().num_days_from_monday();
    let week_start = today - Duration::days(days_since_monday as i64);

    // Go back 51 more weeks (52 total)
    let grid_start = week_start - Duration::weeks(51);

    // Build grid: 7 rows (Mon-Sun) x 52 columns (weeks)
    // Using explicit indexing here for clarity: grid[day_idx][week_idx]
    let mut grid: Vec<Vec<Option<HeatmapCell>>> = vec![vec![None; 52]; 7];

    #[allow(clippy::needless_range_loop)]
    for week_idx in 0..52 {
        for day_idx in 0..7 {
            let date =
                grid_start + Duration::weeks(week_idx as i64) + Duration::days(day_idx as i64);

            // Skip future dates
            if date > today {
                continue;
            }

            let tokens = token_map.get(&date).copied().unwrap_or(0);
            let intensity = percentiles
                .map(|p| p.intensity(tokens))
                .unwrap_or(HeatmapIntensity::None);

            grid[day_idx][week_idx] = Some(HeatmapCell {
                date,
                tokens,
                intensity,
            });
        }
    }

    grid
}

/// Heatmap widget for ratatui
pub struct Heatmap {
    grid: Vec<Vec<Option<HeatmapCell>>>,
}

impl Heatmap {
    pub fn new(daily_tokens: &[(NaiveDate, u64)], today: NaiveDate) -> Self {
        Self {
            grid: build_grid(daily_tokens, today),
        }
    }
}

impl Widget for Heatmap {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let weekday_labels = ['M', ' ', 'W', ' ', 'F', ' ', 'S'];

        for (row_idx, row) in self.grid.iter().enumerate() {
            if row_idx >= area.height as usize {
                break;
            }

            let y = area.y + row_idx as u16;

            // Draw weekday label
            if area.width > 2 {
                buf.set_string(
                    area.x,
                    y,
                    weekday_labels[row_idx].to_string(),
                    Style::default().fg(Color::DarkGray),
                );
            }

            // Draw heatmap cells
            let start_x = area.x + 2; // After label + space
            for (col_idx, cell) in row.iter().enumerate() {
                let x = start_x + col_idx as u16;
                if x >= area.x + area.width {
                    break;
                }

                if let Some(cell) = cell {
                    let ch = cell.intensity.to_char();
                    let style = Style::default().fg(cell.intensity.color());
                    buf.set_string(x, y, ch.to_string(), style);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    // ========== HeatmapIntensity tests ==========

    #[test]
    fn test_intensity_to_char() {
        assert_eq!(HeatmapIntensity::None.to_char(), ' ');
        assert_eq!(HeatmapIntensity::Low.to_char(), '░');
        assert_eq!(HeatmapIntensity::Medium.to_char(), '▒');
        assert_eq!(HeatmapIntensity::High.to_char(), '▓');
        assert_eq!(HeatmapIntensity::Max.to_char(), '█');
    }

    #[test]
    fn test_intensity_color() {
        assert_eq!(HeatmapIntensity::None.color(), Color::DarkGray);
        assert_eq!(HeatmapIntensity::Low.color(), Color::Green);
        assert_eq!(HeatmapIntensity::Medium.color(), Color::Yellow);
        assert_eq!(HeatmapIntensity::High.color(), Color::Rgb(255, 165, 0));
        assert_eq!(HeatmapIntensity::Max.color(), Color::Red);
    }

    // ========== calculate_percentiles tests ==========

    #[test]
    fn test_calculate_percentiles_empty() {
        let result = calculate_percentiles(&[]);
        assert!(result.is_none());
    }

    #[test]
    fn test_calculate_percentiles_all_zeros() {
        let result = calculate_percentiles(&[0, 0, 0]);
        assert!(result.is_none());
    }

    #[test]
    fn test_calculate_percentiles_single_value() {
        let result = calculate_percentiles(&[100]).unwrap();
        assert_eq!(result.p25, 100);
        assert_eq!(result.p50, 100);
        assert_eq!(result.p75, 100);
    }

    #[test]
    fn test_calculate_percentiles_four_values() {
        // [10, 20, 30, 40] sorted
        let result = calculate_percentiles(&[40, 10, 30, 20]).unwrap();
        assert_eq!(result.p25, 10); // 25% of 4 = 1 -> index 0
        assert_eq!(result.p50, 20); // 50% of 4 = 2 -> index 1
        assert_eq!(result.p75, 30); // 75% of 4 = 3 -> index 2
    }

    #[test]
    fn test_calculate_percentiles_ignores_zeros() {
        let result = calculate_percentiles(&[0, 100, 0, 200, 0, 300, 0, 400]).unwrap();
        // Non-zero: [100, 200, 300, 400]
        assert_eq!(result.p25, 100);
        assert_eq!(result.p50, 200);
        assert_eq!(result.p75, 300);
    }

    // ========== Percentiles::to_intensity tests ==========

    #[test]
    fn test_intensity_mapping() {
        let p = Percentiles {
            p25: 100,
            p50: 200,
            p75: 300,
        };

        assert_eq!(p.intensity(0), HeatmapIntensity::None);
        assert_eq!(p.intensity(50), HeatmapIntensity::Low);
        assert_eq!(p.intensity(100), HeatmapIntensity::Low);
        assert_eq!(p.intensity(150), HeatmapIntensity::Medium);
        assert_eq!(p.intensity(200), HeatmapIntensity::Medium);
        assert_eq!(p.intensity(250), HeatmapIntensity::High);
        assert_eq!(p.intensity(300), HeatmapIntensity::High);
        assert_eq!(p.intensity(400), HeatmapIntensity::Max);
    }

    // ========== build_grid tests ==========

    #[test]
    fn test_build_grid_dimensions() {
        let today = NaiveDate::from_ymd_opt(2024, 6, 15).unwrap(); // Saturday
        let daily_tokens = vec![];

        let grid = build_grid(&daily_tokens, today);

        // Should be 7 rows (weekdays)
        assert_eq!(grid.len(), 7);
        // Each row should have 52 columns (weeks)
        for row in &grid {
            assert_eq!(row.len(), 52);
        }
    }

    #[test]
    fn test_build_grid_with_data() {
        let today = NaiveDate::from_ymd_opt(2024, 6, 15).unwrap();
        let daily_tokens = vec![
            (NaiveDate::from_ymd_opt(2024, 6, 15).unwrap(), 1000),
            (NaiveDate::from_ymd_opt(2024, 6, 14).unwrap(), 500),
        ];

        let grid = build_grid(&daily_tokens, today);

        // Find today's cell and verify it has data
        let mut found = false;
        for row in &grid {
            for cell in row.iter().flatten() {
                if cell.date == today {
                    assert_eq!(cell.tokens, 1000);
                    found = true;
                }
            }
        }
        assert!(found, "Today's cell should be in the grid");
    }

    #[test]
    fn test_build_grid_future_dates_excluded() {
        let today = NaiveDate::from_ymd_opt(2024, 6, 12).unwrap(); // Wednesday
        let daily_tokens = vec![];

        let grid = build_grid(&daily_tokens, today);

        // Future dates (Thu, Fri, Sat, Sun of current week) should be None
        for row in &grid {
            for cell in row.iter().flatten() {
                assert!(cell.date <= today, "Grid should not contain future dates");
            }
        }
    }
}
