//! Models view widget - displays per-model usage statistics

use std::collections::HashMap;

use ratatui::{
    buffer::Buffer,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Widget},
};

use super::overview::format_number;
use super::tabs::{Tab, TabBar};
use crate::types::ModelUsage;

/// Format a percentage bar with filled/empty blocks
/// Example: 50.0% with width 10 → "█████░░░░░"
pub fn format_percentage_bar(percent: f64, width: usize) -> String {
    let filled = ((percent / 100.0) * width as f64).round() as usize;
    let empty = width.saturating_sub(filled);
    format!("{}{}", "█".repeat(filled), "░".repeat(empty))
}

/// Model summary for display (pre-sorted)
#[derive(Debug, Clone)]
pub struct ModelSummary {
    pub name: String,
    pub total_tokens: u64,
    pub cost_usd: f64,
}

/// Data for the models view
#[derive(Debug)]
pub struct ModelsData {
    /// Models sorted by cost descending
    pub models: Vec<ModelSummary>,
    /// Total cost across all models (for percentage calculation)
    pub total_cost: f64,
}

impl ModelsData {
    /// Create ModelsData from Aggregator::by_model() output
    pub fn from_model_usage(model_map: &HashMap<String, ModelUsage>) -> Self {
        let total_cost: f64 = model_map.values().map(|m| m.cost_usd).sum();

        let mut models: Vec<ModelSummary> = model_map
            .iter()
            .map(|(name, usage)| {
                let total_tokens = usage.input_tokens
                    + usage.output_tokens
                    + usage.cache_read_tokens
                    + usage.cache_creation_tokens;
                ModelSummary {
                    name: name.clone(),
                    total_tokens,
                    cost_usd: usage.cost_usd,
                }
            })
            .collect();

        // Sort by cost descending (NaN-safe)
        models.sort_by(|a, b| {
            b.cost_usd
                .partial_cmp(&a.cost_usd)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        Self { models, total_cost }
    }
}

/// Maximum content width for Models view (consistent with Overview)
const MAX_CONTENT_WIDTH: u16 = 170;

/// Table width: Model(30) + Tokens(18) + Cost(12) + Usage(18) = 78
const TABLE_WIDTH: u16 = 78;

/// Models view widget
pub struct ModelsView<'a> {
    data: &'a ModelsData,
    selected_tab: Tab,
}

impl<'a> ModelsView<'a> {
    pub fn new(data: &'a ModelsData) -> Self {
        Self {
            data,
            selected_tab: Tab::Models,
        }
    }

    pub fn with_tab(mut self, tab: Tab) -> Self {
        self.selected_tab = tab;
        self
    }
}

impl Widget for ModelsView<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        // Apply max width constraint and center the content
        let content_width = area.width.min(MAX_CONTENT_WIDTH);
        let x_offset = (area.width.saturating_sub(content_width)) / 2;
        let centered_area = Rect {
            x: area.x + x_offset,
            y: area.y,
            width: content_width,
            height: area.height,
        };

        // Calculate layout with models list
        let max_model_rows = self.data.models.len().min(10) as u16; // Show up to 10 models
        let chunks = Layout::vertical([
            Constraint::Length(1),              // Top padding
            Constraint::Length(1),              // Tabs
            Constraint::Length(1),              // Separator
            Constraint::Length(1),              // Header
            Constraint::Length(max_model_rows), // Model rows
            Constraint::Length(1),              // Separator
            Constraint::Length(1),              // Keybindings
            Constraint::Min(0),                 // Remaining space
        ])
        .split(centered_area);

        // Render tabs
        self.render_tabs(chunks[1], buf);

        // Render separator
        self.render_separator(chunks[2], buf);

        // Render header
        self.render_header(chunks[3], buf);

        // Render model rows
        self.render_models(chunks[4], buf);

        // Render separator
        self.render_separator(chunks[5], buf);

        // Render keybindings
        self.render_keybindings(chunks[6], buf);
    }
}

impl ModelsView<'_> {
    /// Calculate horizontal offset to center the table
    fn calculate_table_offset(&self, area_width: u16) -> u16 {
        area_width.saturating_sub(TABLE_WIDTH) / 2
    }

    fn render_tabs(&self, area: Rect, buf: &mut Buffer) {
        let tab_bar = TabBar::new(self.selected_tab);
        tab_bar.render(area, buf);
    }

    fn render_separator(&self, area: Rect, buf: &mut Buffer) {
        let line = "─".repeat(area.width as usize);
        buf.set_string(area.x, area.y, &line, Style::default().fg(Color::DarkGray));
    }

    fn render_header(&self, area: Rect, buf: &mut Buffer) {
        let offset = self.calculate_table_offset(area.width);

        // Column widths: Model(30), Tokens(18), Cost(12), Usage(18)
        let header = Line::from(vec![
            Span::styled(
                format!("{:<30}", "Model"),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("{:>18}", "Tokens"),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("{:>12}", "Cost"),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("{:>18}", "Usage"),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
        ]);

        let paragraph = Paragraph::new(header).alignment(Alignment::Left);
        paragraph.render(
            Rect {
                x: area.x + offset,
                y: area.y,
                width: TABLE_WIDTH.min(area.width),
                height: area.height,
            },
            buf,
        );
    }

    fn render_models(&self, area: Rect, buf: &mut Buffer) {
        let offset = self.calculate_table_offset(area.width);

        for (i, model) in self
            .data
            .models
            .iter()
            .take(area.height as usize)
            .enumerate()
        {
            let y = area.y + i as u16;
            if y >= area.y + area.height {
                break;
            }

            let percent = if self.data.total_cost > 0.0 {
                (model.cost_usd / self.data.total_cost) * 100.0
            } else {
                0.0
            };

            let bar = format_percentage_bar(percent, 14);

            // Truncate model name if too long (UTF-8 safe)
            let name = if model.name.chars().count() > 28 {
                format!("{}…", model.name.chars().take(27).collect::<String>())
            } else {
                model.name.clone()
            };

            let row = Line::from(vec![
                Span::styled(format!("{:<30}", name), Style::default().fg(Color::Cyan)),
                Span::styled(
                    format!("{:>18}", format_number(model.total_tokens)),
                    Style::default().fg(Color::White),
                ),
                Span::styled(
                    format!("{:>12}", format!("${:.2}", model.cost_usd)),
                    Style::default().fg(Color::Magenta),
                ),
                Span::styled(format!("{:>18}", bar), Style::default().fg(Color::Green)),
            ]);

            let paragraph = Paragraph::new(row).alignment(Alignment::Left);
            paragraph.render(
                Rect {
                    x: area.x + offset,
                    y,
                    width: TABLE_WIDTH.min(area.width),
                    height: 1,
                },
                buf,
            );
        }
    }

    fn render_keybindings(&self, area: Rect, buf: &mut Buffer) {
        let bindings = Paragraph::new(Line::from(vec![
            Span::styled("q", Style::default().fg(Color::Cyan)),
            Span::styled(": Quit", Style::default().fg(Color::DarkGray)),
            Span::raw("  "),
            Span::styled("Tab", Style::default().fg(Color::Cyan)),
            Span::styled(": Switch view", Style::default().fg(Color::DarkGray)),
            Span::raw("  "),
            Span::styled("?", Style::default().fg(Color::Cyan)),
            Span::styled(": Help", Style::default().fg(Color::DarkGray)),
        ]))
        .alignment(Alignment::Center);

        bindings.render(area, buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ========== format_percentage_bar tests ==========

    #[test]
    fn test_format_percentage_bar_zero() {
        assert_eq!(format_percentage_bar(0.0, 10), "░░░░░░░░░░");
    }

    #[test]
    fn test_format_percentage_bar_fifty() {
        assert_eq!(format_percentage_bar(50.0, 10), "█████░░░░░");
    }

    #[test]
    fn test_format_percentage_bar_hundred() {
        assert_eq!(format_percentage_bar(100.0, 10), "██████████");
    }

    #[test]
    fn test_format_percentage_bar_twenty_five() {
        // 25% of 8 = 2 filled
        assert_eq!(format_percentage_bar(25.0, 8), "██░░░░░░");
    }

    #[test]
    fn test_format_percentage_bar_rounding() {
        // 33% of 10 = 3.3 → rounds to 3
        assert_eq!(format_percentage_bar(33.0, 10), "███░░░░░░░");
    }

    // ========== ModelsData tests ==========

    #[test]
    fn test_models_data_empty() {
        let model_map: HashMap<String, ModelUsage> = HashMap::new();
        let data = ModelsData::from_model_usage(&model_map);

        assert!(data.models.is_empty());
        assert!((data.total_cost - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_models_data_single_model() {
        let mut model_map: HashMap<String, ModelUsage> = HashMap::new();
        model_map.insert(
            "claude-sonnet-4".to_string(),
            ModelUsage {
                input_tokens: 1000,
                output_tokens: 500,
                cache_read_tokens: 100,
                cache_creation_tokens: 50,
                cost_usd: 0.05,
                count: 10,
            },
        );

        let data = ModelsData::from_model_usage(&model_map);

        assert_eq!(data.models.len(), 1);
        assert_eq!(data.models[0].name, "claude-sonnet-4");
        assert_eq!(data.models[0].total_tokens, 1650); // 1000+500+100+50
        assert!((data.models[0].cost_usd - 0.05).abs() < f64::EPSILON);
        assert!((data.total_cost - 0.05).abs() < f64::EPSILON);
    }

    #[test]
    fn test_models_data_sorted_by_cost_descending() {
        let mut model_map: HashMap<String, ModelUsage> = HashMap::new();
        model_map.insert(
            "claude-haiku".to_string(),
            ModelUsage {
                input_tokens: 100,
                output_tokens: 50,
                cache_read_tokens: 0,
                cache_creation_tokens: 0,
                cost_usd: 0.01,
                count: 1,
            },
        );
        model_map.insert(
            "claude-opus".to_string(),
            ModelUsage {
                input_tokens: 1000,
                output_tokens: 500,
                cache_read_tokens: 0,
                cache_creation_tokens: 0,
                cost_usd: 0.50,
                count: 5,
            },
        );
        model_map.insert(
            "claude-sonnet".to_string(),
            ModelUsage {
                input_tokens: 500,
                output_tokens: 250,
                cache_read_tokens: 0,
                cache_creation_tokens: 0,
                cost_usd: 0.10,
                count: 3,
            },
        );

        let data = ModelsData::from_model_usage(&model_map);

        assert_eq!(data.models.len(), 3);
        // Should be sorted by cost descending: opus (0.50) > sonnet (0.10) > haiku (0.01)
        assert_eq!(data.models[0].name, "claude-opus");
        assert_eq!(data.models[1].name, "claude-sonnet");
        assert_eq!(data.models[2].name, "claude-haiku");
    }

    #[test]
    fn test_models_data_total_cost() {
        let mut model_map: HashMap<String, ModelUsage> = HashMap::new();
        model_map.insert(
            "model-a".to_string(),
            ModelUsage {
                input_tokens: 100,
                output_tokens: 50,
                cache_read_tokens: 0,
                cache_creation_tokens: 0,
                cost_usd: 0.10,
                count: 1,
            },
        );
        model_map.insert(
            "model-b".to_string(),
            ModelUsage {
                input_tokens: 200,
                output_tokens: 100,
                cache_read_tokens: 0,
                cache_creation_tokens: 0,
                cost_usd: 0.20,
                count: 1,
            },
        );

        let data = ModelsData::from_model_usage(&model_map);

        assert!((data.total_cost - 0.30).abs() < f64::EPSILON);
    }
}
