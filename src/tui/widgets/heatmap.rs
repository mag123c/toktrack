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
    /// Convert intensity to display character (legacy, kept for potential future use)
    #[allow(dead_code)]
    pub fn to_char(self) -> char {
        match self {
            Self::None => ' ',
            Self::Low => '░',
            Self::Medium => '▒',
            Self::High => '▓',
            Self::Max => '█',
        }
    }

    /// Convert intensity to 3-character cell (2 blocks + 1 space for gap)
    /// Uses distinct block characters for colorblind accessibility
    #[allow(dead_code)]
    pub fn to_cell_str(self) -> &'static str {
        match self {
            Self::None => "░░ ",   // Light shade - empty/no usage
            Self::Low => "▒▒ ",    // Medium shade - low usage
            Self::Medium => "▓▓ ", // Dark shade - medium usage
            Self::High => "██ ",   // Full block - high usage
            Self::Max => "▀▀ ",    // Upper half block - max usage (distinct pattern)
        }
    }

    /// Get color for this intensity (GitHub-style green gradient using ANSI 256)
    pub fn color(self) -> Color {
        match self {
            Self::None => Color::Indexed(236),  // Dark gray (empty cell)
            Self::Low => Color::Indexed(22),    // DarkGreen
            Self::Medium => Color::Indexed(28), // Green4
            Self::High => Color::Indexed(34),   // Green3
            Self::Max => Color::Indexed(40),    // Green3 (bright)
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

/// Build a 7xN grid of heatmap cells (rows = weekdays, cols = weeks)
/// Fills from today going back `weeks_to_show` weeks
pub fn build_grid(
    daily_tokens: &[(NaiveDate, u64)],
    today: NaiveDate,
    weeks_to_show: usize,
) -> Vec<Vec<Option<HeatmapCell>>> {
    use chrono::{Datelike, Duration};

    // Single iteration: build both token_map and all_values together
    let mut token_map = std::collections::HashMap::with_capacity(daily_tokens.len());
    let mut all_values = Vec::with_capacity(daily_tokens.len());
    for &(date, tokens) in daily_tokens {
        token_map.insert(date, tokens);
        all_values.push(tokens);
    }

    // Calculate percentiles for intensity mapping
    let percentiles = calculate_percentiles(&all_values);

    // Find the start of the current week (Monday)
    let days_since_monday = today.weekday().num_days_from_monday();
    let week_start = today - Duration::days(days_since_monday as i64);

    // Go back (weeks_to_show - 1) more weeks
    let grid_start = week_start - Duration::weeks((weeks_to_show - 1) as i64);

    // Build grid: 7 rows (Mon-Sun) x weeks_to_show columns
    let mut grid: Vec<Vec<Option<HeatmapCell>>> = vec![vec![None; weeks_to_show]; 7];

    #[allow(clippy::needless_range_loop)]
    for week_idx in 0..weeks_to_show {
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

/// Cell dimensions for grid layout (no borders)
const CELL_WIDTH: u16 = 2; // 2 chars content (no border)
const LABEL_WIDTH: u16 = 4; // "Mon " prefix

/// Heatmap widget for ratatui
pub struct Heatmap {
    grid: Vec<Vec<Option<HeatmapCell>>>,
    weeks_to_show: usize,
}

impl Heatmap {
    pub fn new(daily_tokens: &[(NaiveDate, u64)], today: NaiveDate, weeks_to_show: usize) -> Self {
        Self {
            grid: build_grid(daily_tokens, today, weeks_to_show),
            weeks_to_show,
        }
    }

    /// Compute weeks to show based on terminal width
    /// Returns weeks count for responsive layout (2-char cells, no borders)
    pub fn weeks_for_width(width: u16) -> usize {
        // Account for label only (no border)
        let available = width.saturating_sub(LABEL_WIDTH);
        let max_weeks = (available / CELL_WIDTH) as usize;

        if max_weeks >= 52 {
            52
        } else if max_weeks >= 26 {
            26
        } else {
            13
        }
    }

    /// Calculate x_offset for centering the heatmap
    fn calculate_x_offset(&self, area: Rect) -> u16 {
        let heatmap_width = LABEL_WIDTH + (self.weeks_to_show as u16 * CELL_WIDTH);
        area.width.saturating_sub(heatmap_width) / 2
    }

    /// Render a content row: Mon ████████
    fn render_content_row(
        &self,
        area: Rect,
        buf: &mut Buffer,
        y: u16,
        day_idx: usize,
        label: &str,
        x_offset: u16,
    ) {
        let start_x = area.x + x_offset + LABEL_WIDTH;
        let max_x = area.x + area.width;

        // Draw weekday label
        buf.set_string(
            area.x + x_offset,
            y,
            label,
            Style::default().fg(Color::DarkGray),
        );

        // Cells (no borders)
        let row = &self.grid[day_idx];
        for (col_idx, cell) in row.iter().enumerate() {
            if col_idx >= self.weeks_to_show {
                break;
            }
            let x = start_x + (col_idx as u16 * CELL_WIDTH);
            if x + 2 > max_x {
                break;
            }

            // Cell content (2 chars)
            if let Some(cell) = cell {
                let style = Style::default().fg(cell.intensity.color());
                buf.set_string(x, y, "██", style);
            }
        }
    }
}

/// Rows to display in the heatmap (all 7 days: Mon-Sun)
const DISPLAY_ROWS: [(usize, &str); 7] = [
    (0, "Mon"),
    (1, "Tue"),
    (2, "Wed"),
    (3, "Thu"),
    (4, "Fri"),
    (5, "Sat"),
    (6, "Sun"),
];

impl Widget for Heatmap {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let x_offset = self.calculate_x_offset(area);
        let start_x = area.x + x_offset + LABEL_WIDTH;

        // Render 7 rows (Mon-Sun) directly, no borders
        for (day_idx, (_, label)) in DISPLAY_ROWS.iter().enumerate() {
            let y = area.y + day_idx as u16;
            if y >= area.y + area.height {
                break;
            }

            // Content row: Mon ████████
            self.render_content_row(area, buf, y, day_idx, label, x_offset);
        }

        // Render month labels below the grid (after 7 rows)
        let month_label_y = area.y + 7;
        if month_label_y < area.y + area.height && !self.grid[0].is_empty() {
            self.render_month_labels(area, buf, start_x, month_label_y, CELL_WIDTH);
        }
    }
}

impl Heatmap {
    /// Render month labels below the heatmap grid
    fn render_month_labels(
        &self,
        area: Rect,
        buf: &mut Buffer,
        start_x: u16,
        y: u16,
        cell_width: u16,
    ) {
        use chrono::Datelike;

        let mut last_month: Option<u32> = None;
        let month_names = [
            "", "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
        ];

        for (col_idx, cell) in self.grid[0].iter().enumerate() {
            if col_idx >= self.weeks_to_show {
                break;
            }
            let x = start_x + (col_idx as u16 * cell_width);
            if x + 3 > area.x + area.width {
                break;
            }

            if let Some(cell) = cell {
                let month = cell.date.month();
                if last_month.is_none_or(|m| m != month) {
                    let label = month_names[month as usize];
                    buf.set_string(x, y, label, Style::default().fg(Color::DarkGray));
                    last_month = Some(month);
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
    fn test_intensity_to_cell_str() {
        // Each intensity uses distinct block characters for colorblind accessibility
        assert_eq!(HeatmapIntensity::None.to_cell_str(), "░░ ");
        assert_eq!(HeatmapIntensity::Low.to_cell_str(), "▒▒ ");
        assert_eq!(HeatmapIntensity::Medium.to_cell_str(), "▓▓ ");
        assert_eq!(HeatmapIntensity::High.to_cell_str(), "██ ");
        assert_eq!(HeatmapIntensity::Max.to_cell_str(), "▀▀ ");
    }

    #[test]
    fn test_intensity_color() {
        // GitHub-style green gradient using ANSI 256 colors
        assert_eq!(HeatmapIntensity::None.color(), Color::Indexed(236)); // dark gray
        assert_eq!(HeatmapIntensity::Low.color(), Color::Indexed(22)); // DarkGreen
        assert_eq!(HeatmapIntensity::Medium.color(), Color::Indexed(28)); // Green4
        assert_eq!(HeatmapIntensity::High.color(), Color::Indexed(34)); // Green3
        assert_eq!(HeatmapIntensity::Max.color(), Color::Indexed(40)); // Green3 (bright)
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

        let grid = build_grid(&daily_tokens, today, 52);

        // Should be 7 rows (weekdays)
        assert_eq!(grid.len(), 7);
        // Each row should have 52 columns (weeks)
        for row in &grid {
            assert_eq!(row.len(), 52);
        }
    }

    #[test]
    fn test_build_grid_26_weeks() {
        let today = NaiveDate::from_ymd_opt(2024, 6, 15).unwrap();
        let daily_tokens = vec![];

        let grid = build_grid(&daily_tokens, today, 26);

        assert_eq!(grid.len(), 7);
        for row in &grid {
            assert_eq!(row.len(), 26);
        }
    }

    #[test]
    fn test_build_grid_13_weeks() {
        let today = NaiveDate::from_ymd_opt(2024, 6, 15).unwrap();
        let daily_tokens = vec![];

        let grid = build_grid(&daily_tokens, today, 13);

        assert_eq!(grid.len(), 7);
        for row in &grid {
            assert_eq!(row.len(), 13);
        }
    }

    #[test]
    fn test_build_grid_with_data() {
        let today = NaiveDate::from_ymd_opt(2024, 6, 15).unwrap();
        let daily_tokens = vec![
            (NaiveDate::from_ymd_opt(2024, 6, 15).unwrap(), 1000),
            (NaiveDate::from_ymd_opt(2024, 6, 14).unwrap(), 500),
        ];

        let grid = build_grid(&daily_tokens, today, 52);

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

        let grid = build_grid(&daily_tokens, today, 52);

        // Future dates (Thu, Fri, Sat, Sun of current week) should be None
        for row in &grid {
            for cell in row.iter().flatten() {
                assert!(cell.date <= today, "Grid should not contain future dates");
            }
        }
    }

    // ========== weeks_for_width tests ==========

    #[test]
    fn test_weeks_for_width_wide() {
        // 52 weeks needs: label 4 + 52*2 = 108 (2-char cells, no borders)
        // So width >= 108 -> 52 weeks
        assert_eq!(Heatmap::weeks_for_width(108), 52);
        assert_eq!(Heatmap::weeks_for_width(120), 52);
        assert_eq!(Heatmap::weeks_for_width(200), 52);
    }

    #[test]
    fn test_weeks_for_width_medium() {
        // 26 weeks needs: label 4 + 26*2 = 56
        // So width 56-107 -> 26 weeks
        assert_eq!(Heatmap::weeks_for_width(56), 26);
        assert_eq!(Heatmap::weeks_for_width(80), 26);
        assert_eq!(Heatmap::weeks_for_width(107), 26);
    }

    #[test]
    fn test_weeks_for_width_narrow() {
        // 13 weeks needs: label 4 + 13*2 = 30
        // So width < 56 -> 13 weeks
        assert_eq!(Heatmap::weeks_for_width(30), 13);
        assert_eq!(Heatmap::weeks_for_width(55), 13);
    }

    // ========== CELL_WIDTH tests ==========

    #[test]
    fn test_cell_dimensions() {
        // Cell should be 2 chars wide (no border)
        assert_eq!(CELL_WIDTH, 2);
        // Label width should be 4 chars ("Mon ")
        assert_eq!(LABEL_WIDTH, 4);
    }

    // ========== Grid rendering tests (borderless) ==========

    /// Helper to create a test buffer and heatmap
    fn create_test_heatmap(weeks: usize) -> (Heatmap, Rect, Buffer) {
        let today = NaiveDate::from_ymd_opt(2024, 6, 15).unwrap();
        let daily_tokens = vec![
            (NaiveDate::from_ymd_opt(2024, 6, 15).unwrap(), 1000),
            (NaiveDate::from_ymd_opt(2024, 6, 14).unwrap(), 500),
        ];
        let heatmap = Heatmap::new(&daily_tokens, today, weeks);

        // Create area large enough for grid: label(4) + weeks*2
        let width = LABEL_WIDTH + (weeks as u16 * CELL_WIDTH);
        let height = 8; // 7 rows + 1 month label
        let area = Rect::new(0, 0, width, height);
        let buf = Buffer::empty(area);

        (heatmap, area, buf)
    }

    #[test]
    fn test_render_content_row_has_label() {
        let (heatmap, area, mut buf) = create_test_heatmap(3);

        // x_offset is 0 when buffer width equals heatmap width
        heatmap.render_content_row(area, &mut buf, 0, 0, "Mon", 0);

        // Check label at x=0
        let cell = buf.cell((0, 0)).unwrap();
        assert_eq!(cell.symbol(), "M");
        let cell = buf.cell((1, 0)).unwrap();
        assert_eq!(cell.symbol(), "o");
        let cell = buf.cell((2, 0)).unwrap();
        assert_eq!(cell.symbol(), "n");
    }

    #[test]
    fn test_render_content_row_no_borders() {
        let (heatmap, area, mut buf) = create_test_heatmap(3);

        // x_offset is 0 when buffer width equals heatmap width
        heatmap.render_content_row(area, &mut buf, 0, 0, "Mon", 0);

        let start_x = LABEL_WIDTH as usize;

        // Cell content should be directly at start_x (no left border)
        let cell = buf.cell((start_x as u16, 0)).unwrap();
        // Should be a block char from intensity, not a border
        assert_ne!(cell.symbol(), "│");
    }

    #[test]
    fn test_full_grid_structure_borderless() {
        let (heatmap, area, mut buf) = create_test_heatmap(3);

        // Render full heatmap
        heatmap.render(area, &mut buf);

        // Row 0: Mon content row - should have label
        assert_eq!(buf.cell((0, 0)).unwrap().symbol(), "M");
        assert_eq!(buf.cell((1, 0)).unwrap().symbol(), "o");
        assert_eq!(buf.cell((2, 0)).unwrap().symbol(), "n");

        // Row 6: Sun content row - should have label
        assert_eq!(buf.cell((0, 6)).unwrap().symbol(), "S");
        assert_eq!(buf.cell((1, 6)).unwrap().symbol(), "u");
        assert_eq!(buf.cell((2, 6)).unwrap().symbol(), "n");
    }
}
