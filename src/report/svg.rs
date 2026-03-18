//! SVG renderer for usage reports (receipt style)

use std::fmt::Write;

use crate::report::data::ReportData;

const WIDTH: u32 = 600;
const PADDING: u32 = 30;
const LINE_HEIGHT: u32 = 24;
const SECTION_GAP: u32 = 16;
const BAR_HEIGHT: u32 = 14;
const BAR_MAX_W: u32 = 160;
const INNER_W: u32 = WIDTH - PADDING * 2;

const COLORS: &[&str] = &[
    "#4F46E5", // indigo
    "#0EA5E9", // sky
    "#10B981", // emerald
    "#F59E0B", // amber
    "#EF4444", // red
    "#8B5CF6", // violet
];

fn color_for(index: usize) -> &'static str {
    COLORS[index % COLORS.len()]
}

fn estimate_height(data: &ReportData) -> u32 {
    let mut h: u32 = PADDING;
    h += LINE_HEIGHT * 3 + SECTION_GAP; // header
    h += LINE_HEIGHT * 4 + SECTION_GAP; // summary

    if !data.by_model.is_empty() {
        h += LINE_HEIGHT * (1 + data.by_model.len() as u32) + SECTION_GAP;
    }
    if !data.by_source.is_empty() {
        h += LINE_HEIGHT * (1 + data.by_source.len() as u32) + SECTION_GAP;
    }
    if !data.daily.is_empty() {
        h += LINE_HEIGHT * (1 + data.daily.len() as u32) + SECTION_GAP;
    }
    if data.most_expensive_day.is_some() {
        h += LINE_HEIGHT + SECTION_GAP;
    }
    h += LINE_HEIGHT * 2 + PADDING; // footer
    h
}

fn format_tokens(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}K", n as f64 / 1_000.0)
    } else {
        format!("{}", n)
    }
}

fn format_cost(cost: f64) -> String {
    format!("${:.2}", cost)
}

fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Render report data as SVG string
pub fn render(data: &ReportData) -> String {
    let height = estimate_height(data);
    let mut svg = String::with_capacity(4096);

    // SVG header + styles
    write!(
        svg,
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{WIDTH}" height="{height}" viewBox="0 0 {WIDTH} {height}">
<defs>
  <style>
    .bg {{ fill: #ffffff; }}
    .title {{ font-family: 'SF Mono', 'Menlo', 'Consolas', monospace; font-size: 18px; font-weight: bold; fill: #111827; }}
    .subtitle {{ font-family: 'SF Mono', 'Menlo', 'Consolas', monospace; font-size: 12px; fill: #6B7280; }}
    .label {{ font-family: 'SF Mono', 'Menlo', 'Consolas', monospace; font-size: 13px; fill: #374151; }}
    .value {{ font-family: 'SF Mono', 'Menlo', 'Consolas', monospace; font-size: 13px; fill: #111827; font-weight: bold; }}
    .section {{ font-family: 'SF Mono', 'Menlo', 'Consolas', monospace; font-size: 11px; fill: #9CA3AF; font-weight: bold; letter-spacing: 1px; }}
    .footer {{ font-family: 'SF Mono', 'Menlo', 'Consolas', monospace; font-size: 10px; fill: #9CA3AF; }}
    .bar {{ rx: 3; ry: 3; }}
    .separator {{ stroke: #E5E7EB; stroke-dasharray: 4,4; }}
  </style>
</defs>
<rect class="bg" width="{WIDTH}" height="{height}" rx="12" ry="12"/>
<rect x="1" y="1" width="{w2}" height="{h2}" rx="12" ry="12" fill="none" stroke="#E5E7EB" stroke-width="1"/>"##,
        w2 = WIDTH - 2,
        h2 = height - 2,
    )
    .unwrap();

    let mut y = PADDING;
    let x = PADDING;
    let right = WIDTH - PADDING;
    let cx = WIDTH / 2;

    // Title
    y += LINE_HEIGHT;
    write!(
        svg,
        r#"<text x="{cx}" y="{y}" class="title" text-anchor="middle">AI CODING RECEIPT</text>"#,
    )
    .unwrap();

    // Period
    y += LINE_HEIGHT;
    write!(
        svg,
        r#"<text x="{cx}" y="{y}" class="subtitle" text-anchor="middle">{}</text>"#,
        escape_xml(&data.period_label),
    )
    .unwrap();

    // Separator
    y += SECTION_GAP;
    svg_separator(&mut svg, x, right, y);

    // Summary
    y += LINE_HEIGHT;
    svg_label_value(
        &mut svg,
        x,
        right,
        y,
        "Total Cost",
        &format_cost(data.total_cost),
    );
    y += LINE_HEIGHT;
    svg_label_value(
        &mut svg,
        x,
        right,
        y,
        "Input Tokens",
        &format_tokens(data.total_input_tokens),
    );
    y += LINE_HEIGHT;
    svg_label_value(
        &mut svg,
        x,
        right,
        y,
        "Output Tokens",
        &format_tokens(data.total_output_tokens),
    );
    y += LINE_HEIGHT;
    svg_label_value(
        &mut svg,
        x,
        right,
        y,
        "Active Days",
        &format!("{}/{}", data.active_days, data.total_days),
    );

    // By Model
    if !data.by_model.is_empty() {
        y += SECTION_GAP;
        svg_separator(&mut svg, x, right, y);
        y += LINE_HEIGHT;
        write!(
            svg,
            r#"<text x="{x}" y="{y}" class="section">BY MODEL</text>"#
        )
        .unwrap();

        let max_cost = data
            .by_model
            .iter()
            .map(|m| m.cost_usd)
            .fold(0.0_f64, f64::max);

        for (i, model) in data.by_model.iter().enumerate() {
            y += LINE_HEIGHT;
            let name = escape_xml(&model.name);
            let cost = format_cost(model.cost_usd);
            let tokens = format_tokens(model.input_tokens.saturating_add(model.output_tokens));

            write!(svg, r#"<text x="{x}" y="{y}" class="label">{name}</text>"#).unwrap();
            write!(
                svg,
                r#"<text x="{mx}" y="{y}" class="value" text-anchor="end">{cost}</text>"#,
                mx = x + 220,
            )
            .unwrap();
            write!(
                svg,
                r#"<text x="{tx}" y="{y}" class="label" text-anchor="end">{tokens}</text>"#,
                tx = x + 290,
            )
            .unwrap();

            let ratio = if max_cost > 0.0 {
                model.cost_usd / max_cost
            } else {
                0.0
            };
            let bar_w = (ratio * BAR_MAX_W as f64).round() as u32;
            if bar_w > 0 {
                let bar_x = x + 300;
                let bar_y = y - BAR_HEIGHT + 3;
                let color = color_for(i);
                write!(
                    svg,
                    r#"<rect x="{bar_x}" y="{bar_y}" width="{bar_w}" height="{BAR_HEIGHT}" class="bar" fill="{color}" opacity="0.8"/>"#,
                )
                .unwrap();
            }
        }
    }

    // By Source
    if !data.by_source.is_empty() {
        y += SECTION_GAP;
        svg_separator(&mut svg, x, right, y);
        y += LINE_HEIGHT;
        write!(
            svg,
            r#"<text x="{x}" y="{y}" class="section">BY SOURCE</text>"#
        )
        .unwrap();

        for source in &data.by_source {
            y += LINE_HEIGHT;
            let name = escape_xml(&source.name);
            let cost = format_cost(source.cost_usd);
            let tokens = format_tokens(source.total_tokens);

            write!(svg, r#"<text x="{x}" y="{y}" class="label">{name}</text>"#).unwrap();
            write!(
                svg,
                r#"<text x="{mx}" y="{y}" class="value" text-anchor="end">{cost}</text>"#,
                mx = x + 220,
            )
            .unwrap();
            write!(
                svg,
                r#"<text x="{tx}" y="{y}" class="label" text-anchor="end">{tokens}</text>"#,
                tx = x + 290,
            )
            .unwrap();
        }
    }

    // Daily breakdown
    if !data.daily.is_empty() {
        y += SECTION_GAP;
        svg_separator(&mut svg, x, right, y);
        y += LINE_HEIGHT;
        write!(
            svg,
            r#"<text x="{x}" y="{y}" class="section">DAILY BREAKDOWN</text>"#
        )
        .unwrap();

        let max_cost = data
            .daily
            .iter()
            .map(|d| d.cost_usd)
            .fold(0.0_f64, f64::max);

        for (i, day) in data.daily.iter().enumerate() {
            y += LINE_HEIGHT;
            let date_str = day.date.format("%m/%d").to_string();
            let cost = format_cost(day.cost_usd);

            write!(
                svg,
                r#"<text x="{x}" y="{y}" class="label">{date_str}</text>"#
            )
            .unwrap();
            write!(
                svg,
                r#"<text x="{mx}" y="{y}" class="value" text-anchor="end">{cost}</text>"#,
                mx = x + 140,
            )
            .unwrap();

            let ratio = if max_cost > 0.0 {
                day.cost_usd / max_cost
            } else {
                0.0
            };
            let bar_w = (ratio * (INNER_W - 180) as f64).round() as u32;
            if bar_w > 0 {
                let bar_x = x + 150;
                let bar_y = y - BAR_HEIGHT + 3;
                let color = color_for(i);
                write!(
                    svg,
                    r#"<rect x="{bar_x}" y="{bar_y}" width="{bar_w}" height="{BAR_HEIGHT}" class="bar" fill="{color}" opacity="0.7"/>"#,
                )
                .unwrap();
            }
        }
    }

    // Most expensive day highlight
    if let Some(ref day) = data.most_expensive_day {
        y += SECTION_GAP;
        svg_separator(&mut svg, x, right, y);
        y += LINE_HEIGHT;
        let highlight = format!(
            "Peak: {} ({})",
            day.date.format("%Y-%m-%d"),
            format_cost(day.cost_usd)
        );
        write!(
            svg,
            r#"<text x="{cx}" y="{y}" class="label" text-anchor="middle">{}</text>"#,
            escape_xml(&highlight),
        )
        .unwrap();
    }

    // Footer
    y += SECTION_GAP;
    svg_separator(&mut svg, x, right, y);
    y += LINE_HEIGHT;
    write!(
        svg,
        r#"<text x="{cx}" y="{y}" class="footer" text-anchor="middle">Generated by toktrack · github.com/mag123c/toktrack</text>"#,
    )
    .unwrap();

    svg.push_str("\n</svg>");
    svg
}

fn svg_separator(svg: &mut String, x: u32, right: u32, y: u32) {
    write!(
        svg,
        r#"<line x1="{x}" y1="{y}" x2="{right}" y2="{y}" class="separator"/>"#,
    )
    .unwrap();
}

fn svg_label_value(svg: &mut String, x: u32, right: u32, y: u32, label: &str, value: &str) {
    write!(
        svg,
        r#"<text x="{x}" y="{y}" class="label">{label}</text><text x="{right}" y="{y}" class="value" text-anchor="end">{value}</text>"#,
    )
    .unwrap();
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
            by_model: vec![ModelReport {
                name: "Opus 4.5".to_string(),
                input_tokens: 800_000,
                output_tokens: 400_000,
                cost_usd: 30.00,
                count: 50,
            }],
            by_source: vec![SourceReport {
                name: "claude".to_string(),
                total_tokens: 1_802_457,
                cost_usd: 42.37,
            }],
            daily: vec![DayReport {
                date: NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
                total_tokens: 300_000,
                cost_usd: 8.50,
            }],
            most_expensive_day: Some(DayReport {
                date: NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
                total_tokens: 300_000,
                cost_usd: 8.50,
            }),
        }
    }

    #[test]
    fn test_valid_svg_structure() {
        let output = render(&sample_data());
        assert!(output.contains("<svg"));
        assert!(output.contains("</svg>"));
        assert!(output.contains("AI CODING RECEIPT"));
        assert!(output.contains("github.com/mag123c/toktrack"));
    }

    #[test]
    fn test_svg_contains_data() {
        let output = render(&sample_data());
        assert!(output.contains("$42.37"));
        assert!(output.contains("Opus 4.5"));
        assert!(output.contains("claude"));
        assert!(output.contains("01/01"));
    }

    #[test]
    fn test_empty_data_no_crash() {
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
        assert!(output.contains("<svg"));
        assert!(output.contains("</svg>"));
    }

    #[test]
    fn test_xml_escaping() {
        let data = ReportData {
            period_label: "test <>&\"".to_string(),
            total_cost: 0.0,
            total_input_tokens: 0,
            total_output_tokens: 0,
            active_days: 0,
            total_days: 1,
            by_model: vec![],
            by_source: vec![],
            daily: vec![],
            most_expensive_day: None,
        };
        let output = render(&data);
        assert!(output.contains("&lt;"));
        assert!(output.contains("&gt;"));
        assert!(output.contains("&amp;"));
    }
}
