//! Terminal text renderer for usage reports

use crate::report::data::ReportData;

/// Format token count: 1234567 → "1.2M", 12345 → "12.3K", 999 → "999"
fn format_tokens(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}K", n as f64 / 1_000.0)
    } else {
        format!("{}", n)
    }
}

/// Format cost: 42.373 → "$42.37"
fn format_cost(cost: f64) -> String {
    format!("${:.2}", cost)
}

/// Render a horizontal bar using block chars
fn render_bar(ratio: f64, max_width: usize) -> String {
    let filled = (ratio * max_width as f64).round() as usize;
    let filled = filled.min(max_width);
    "\u{2588}".repeat(filled)
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max - 1).collect();
        format!("{}\u{2026}", truncated)
    }
}

/// Render report data as a terminal text receipt
pub fn render(data: &ReportData) -> String {
    let w = 48usize;
    let mut out = String::new();

    // Top border
    out.push_str(&format!("\u{256d}{}\u{256e}\n", "\u{2500}".repeat(w)));

    // Header
    center_line(&mut out, w, "AI CODING RECEIPT");
    center_line(&mut out, w, &data.period_label);

    // Separator
    out.push_str(&format!("\u{2502}{}\u{2502}\n", "\u{2508}".repeat(w)));

    // Summary
    kv_line(&mut out, w, "Total Cost", &format_cost(data.total_cost));
    kv_line(
        &mut out,
        w,
        "Input Tokens",
        &format_tokens(data.total_input_tokens),
    );
    kv_line(
        &mut out,
        w,
        "Output Tokens",
        &format_tokens(data.total_output_tokens),
    );
    kv_line(
        &mut out,
        w,
        "Active Days",
        &format!("{}/{}", data.active_days, data.total_days),
    );

    // By Model
    if !data.by_model.is_empty() {
        out.push_str(&format!("\u{2502}{}\u{2502}\n", "\u{2508}".repeat(w)));
        section_title(&mut out, w, "BY MODEL");

        let max_cost = data
            .by_model
            .iter()
            .map(|m| m.cost_usd)
            .fold(0.0_f64, f64::max);
        let bar_width = 16;

        for model in &data.by_model {
            let ratio = if max_cost > 0.0 {
                model.cost_usd / max_cost
            } else {
                0.0
            };
            let bar = render_bar(ratio, bar_width);
            let name = truncate(&model.name, 14);
            let cost = format_cost(model.cost_usd);
            let tokens = format_tokens(model.input_tokens.saturating_add(model.output_tokens));

            // "  name           $cost  tokens bar   "
            let bar_chars = bar.chars().count();
            let remaining = w.saturating_sub(33 + bar_chars);
            out.push_str(&format!(
                "\u{2502}  {:<14} {:>8} {:>6} {}{}\u{2502}\n",
                name,
                cost,
                tokens,
                bar,
                " ".repeat(remaining)
            ));
        }
    }

    // By Source
    if !data.by_source.is_empty() {
        out.push_str(&format!("\u{2502}{}\u{2502}\n", "\u{2508}".repeat(w)));
        section_title(&mut out, w, "BY SOURCE");

        for source in &data.by_source {
            let name = truncate(&source.name, 14);
            let cost = format_cost(source.cost_usd);
            let tokens = format_tokens(source.total_tokens);
            let remaining = w.saturating_sub(32);
            out.push_str(&format!(
                "\u{2502}  {:<14} {:>8} {:>6}{}\u{2502}\n",
                name,
                cost,
                tokens,
                " ".repeat(remaining)
            ));
        }
    }

    // Daily
    if !data.daily.is_empty() {
        out.push_str(&format!("\u{2502}{}\u{2502}\n", "\u{2508}".repeat(w)));
        section_title(&mut out, w, "DAILY BREAKDOWN");

        let max_cost = data
            .daily
            .iter()
            .map(|d| d.cost_usd)
            .fold(0.0_f64, f64::max);
        let bar_width = 16;

        for day in &data.daily {
            let date_str = day.date.format("%m/%d").to_string();
            let cost = format_cost(day.cost_usd);
            let ratio = if max_cost > 0.0 {
                day.cost_usd / max_cost
            } else {
                0.0
            };
            let bar = render_bar(ratio, bar_width);
            let bar_chars = bar.chars().count();
            let remaining = w.saturating_sub(16 + bar_chars + 1);
            out.push_str(&format!(
                "\u{2502}  {} {:>8} {}{}\u{2502}\n",
                date_str,
                cost,
                bar,
                " ".repeat(remaining)
            ));
        }
    }

    // Highlight: most expensive day
    if let Some(ref day) = data.most_expensive_day {
        out.push_str(&format!("\u{2502}{}\u{2502}\n", "\u{2508}".repeat(w)));
        let highlight = format!(
            "Peak: {} ({})",
            day.date.format("%Y-%m-%d"),
            format_cost(day.cost_usd)
        );
        center_line(&mut out, w, &highlight);
    }

    // Footer
    out.push_str(&format!("\u{2502}{}\u{2502}\n", "\u{2508}".repeat(w)));
    center_line(&mut out, w, "Generated by toktrack");

    // Bottom border
    out.push_str(&format!("\u{2570}{}\u{256f}", "\u{2500}".repeat(w)));

    out
}

fn center_line(out: &mut String, w: usize, text: &str) {
    let text_len = text.chars().count();
    let pad_left = w.saturating_sub(text_len) / 2;
    let pad_right = w.saturating_sub(text_len + pad_left);
    out.push_str(&format!(
        "\u{2502}{}{}{}\u{2502}\n",
        " ".repeat(pad_left),
        text,
        " ".repeat(pad_right)
    ));
}

fn section_title(out: &mut String, w: usize, title: &str) {
    let remaining = w.saturating_sub(title.len() + 1);
    out.push_str(&format!(
        "\u{2502} {}{}\u{2502}\n",
        title,
        " ".repeat(remaining)
    ));
}

fn kv_line(out: &mut String, w: usize, key: &str, value: &str) {
    // " key............ value "
    let key_len = key.len();
    let val_len = value.len();
    let dots = w.saturating_sub(key_len + val_len + 3); // 2 spaces + 1 padding
    out.push_str(&format!(
        "\u{2502} {}{} {:>val_w$}\u{2502}\n",
        key,
        ".".repeat(dots),
        value,
        val_w = val_len
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::data::{DayReport, ModelReport, ReportData, SourceReport};
    use chrono::NaiveDate;

    fn sample_data() -> ReportData {
        ReportData {
            period_label: "2024-01-01 ~ 2024-01-07".to_string(),
            total_cost: 42.37,
            total_input_tokens: 1_234_567,
            total_output_tokens: 567_890,
            active_days: 5,
            total_days: 7,
            by_model: vec![
                ModelReport {
                    name: "Opus 4.5".to_string(),
                    input_tokens: 800_000,
                    output_tokens: 400_000,
                    cost_usd: 30.00,
                    count: 50,
                },
                ModelReport {
                    name: "Sonnet 4".to_string(),
                    input_tokens: 434_567,
                    output_tokens: 167_890,
                    cost_usd: 12.37,
                    count: 30,
                },
            ],
            by_source: vec![SourceReport {
                name: "claude".to_string(),
                total_tokens: 1_802_457,
                cost_usd: 42.37,
            }],
            daily: vec![
                DayReport {
                    date: NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
                    total_tokens: 300_000,
                    cost_usd: 8.50,
                },
                DayReport {
                    date: NaiveDate::from_ymd_opt(2024, 1, 2).unwrap(),
                    total_tokens: 500_000,
                    cost_usd: 15.00,
                },
            ],
            most_expensive_day: Some(DayReport {
                date: NaiveDate::from_ymd_opt(2024, 1, 2).unwrap(),
                total_tokens: 500_000,
                cost_usd: 15.00,
            }),
        }
    }

    #[test]
    fn test_render_contains_sections() {
        let output = render(&sample_data());
        assert!(output.contains("AI CODING RECEIPT"));
        assert!(output.contains("2024-01-01 ~ 2024-01-07"));
        assert!(output.contains("$42.37"));
        assert!(output.contains("BY MODEL"));
        assert!(output.contains("Opus 4.5"));
        assert!(output.contains("Sonnet 4"));
        assert!(output.contains("BY SOURCE"));
        assert!(output.contains("claude"));
        assert!(output.contains("DAILY BREAKDOWN"));
        assert!(output.contains("Generated by toktrack"));
    }

    #[test]
    fn test_render_empty_data() {
        let data = ReportData {
            period_label: "2024-01-01 ~ 2024-01-07".to_string(),
            total_cost: 0.0,
            total_input_tokens: 0,
            total_output_tokens: 0,
            active_days: 0,
            total_days: 7,
            by_model: vec![],
            by_source: vec![],
            daily: vec![],
            most_expensive_day: None,
        };
        let output = render(&data);
        assert!(output.contains("AI CODING RECEIPT"));
        assert!(output.contains("$0.00"));
        assert!(!output.contains("BY MODEL"));
    }

    #[test]
    fn test_format_tokens() {
        assert_eq!(format_tokens(999), "999");
        assert_eq!(format_tokens(1_000), "1.0K");
        assert_eq!(format_tokens(12_345), "12.3K");
        assert_eq!(format_tokens(1_234_567), "1.2M");
    }

    #[test]
    fn test_format_cost() {
        assert_eq!(format_cost(0.0), "$0.00");
        assert_eq!(format_cost(42.373), "$42.37");
        assert_eq!(format_cost(1.5), "$1.50");
    }

    #[test]
    fn test_box_drawing_borders() {
        let output = render(&sample_data());
        let lines: Vec<&str> = output.lines().collect();
        // First line starts with ╭
        assert!(lines[0].starts_with('\u{256d}'));
        // Last line starts with ╰ and ends with ╯
        let last = lines.last().unwrap();
        assert!(last.starts_with('\u{2570}'));
        assert!(last.ends_with('\u{256f}'));
        // Middle lines have │ borders
        for line in &lines[1..lines.len() - 1] {
            assert!(
                line.starts_with('\u{2502}'),
                "Missing left border: {}",
                line
            );
            assert!(line.ends_with('\u{2502}'), "Missing right border: {}", line);
        }
    }
}
