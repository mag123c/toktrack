//! Model name normalization service
//!
//! Normalizes model names to a canonical form for consistent pricing lookup
//! and aggregation across different data sources.

/// Convert normalized model name to human-readable display name.
/// Uses dynamic pattern parsing for automatic support of new models.
///
/// # Examples
/// - "claude-opus-4-5" → "Opus 4.5"
/// - "claude-sonnet-4" → "Sonnet 4"
/// - "claude-haiku-4-5" → "Haiku 4.5"
/// - "gpt-4o" → "GPT-4o"
/// - "gpt-4o-mini" → "GPT-4o Mini"
/// - "gpt-4-1" → "GPT-4.1"
/// - "gpt-4-1-mini" → "GPT-4.1 Mini"
/// - "gemini-2-5-pro" → "Gemini 2.5 Pro"
/// - "o1" → "o1", "o4-mini" → "o4 Mini"
/// - "codex-mini-latest" → "Codex Mini"
pub fn display_name(normalized: &str) -> String {
    if normalized.is_empty() {
        return String::new();
    }

    // Claude: claude-{family}-{version} → {Family} {version}
    if let Some(rest) = normalized.strip_prefix("claude-") {
        return parse_claude_name(rest);
    }

    // GPT: gpt-{variant}(-{suffix}) → GPT-{variant}( {Suffix})
    if let Some(rest) = normalized.strip_prefix("gpt-") {
        return parse_gpt_name(rest);
    }

    // Gemini: gemini-{version}-{tier} → Gemini {version} {Tier}
    if let Some(rest) = normalized.strip_prefix("gemini-") {
        return parse_gemini_name(rest);
    }

    // Codex: codex-{variant}(-latest) → Codex {Variant}
    if let Some(rest) = normalized.strip_prefix("codex-") {
        return parse_codex_name(rest);
    }

    // OpenAI o-series: o{N}, o{N}-mini, o{N}-pro, etc.
    if let Some(rest) = normalized.strip_prefix('o') {
        if rest.starts_with(|c: char| c.is_ascii_digit()) {
            return parse_o_series(normalized);
        }
    }

    // Fallback: return as-is
    normalized.to_string()
}

/// Parse Claude model name: {family}-{version} → {Family} {version}
fn parse_claude_name(rest: &str) -> String {
    // Split into family and version parts
    // e.g., "opus-4-5" → family="opus", version="4-5"
    // e.g., "sonnet-4" → family="sonnet", version="4"
    let parts: Vec<&str> = rest.splitn(2, '-').collect();
    if parts.len() < 2 {
        return format!("Claude {}", capitalize(rest));
    }

    let family = capitalize(parts[0]);
    let version = format_version(parts[1]);

    format!("{} {}", family, version)
}

/// Parse GPT model name with minor version support:
/// - "4o" → "GPT-4o"
/// - "4o-mini" → "GPT-4o Mini"
/// - "4-1" → "GPT-4.1"
/// - "4-1-mini" → "GPT-4.1 Mini"
/// - "4-turbo" → "GPT-4 Turbo"
fn parse_gpt_name(rest: &str) -> String {
    let parts: Vec<&str> = rest.split('-').collect();
    let variant = parts[0];
    // Second segment is 1-2 digit number → minor version (e.g., "4-1" → "4.1")
    if parts.len() >= 2
        && !parts[1].is_empty()
        && parts[1].len() <= 2
        && parts[1].chars().all(|c| c.is_ascii_digit())
    {
        let version = format!("{}.{}", variant, parts[1]);
        if parts.len() > 2 {
            let suffix: Vec<String> = parts[2..].iter().map(|p| capitalize(p)).collect();
            format!("GPT-{} {}", version, suffix.join(" "))
        } else {
            format!("GPT-{}", version)
        }
    } else if parts.len() > 1 {
        let suffix: Vec<String> = parts[1..].iter().map(|p| capitalize(p)).collect();
        format!("GPT-{} {}", variant, suffix.join(" "))
    } else {
        format!("GPT-{}", rest)
    }
}

/// Parse Gemini model name: {version}-{tier} → Gemini {version} {Tier}
fn parse_gemini_name(rest: &str) -> String {
    // e.g., "2-5-pro" → "2.5 Pro"
    // e.g., "2-0-flash" → "2.0 Flash"
    // Find the tier (last part that's not a number)
    let parts: Vec<&str> = rest.split('-').collect();
    if parts.len() < 2 {
        return format!("Gemini {}", rest);
    }

    // Find where version ends and tier begins
    // Version parts are numeric, tier is alphabetic
    let mut version_parts = Vec::new();
    let mut tier_parts = Vec::new();

    for part in parts {
        if part.chars().all(|c| c.is_ascii_digit()) && tier_parts.is_empty() {
            version_parts.push(part);
        } else {
            tier_parts.push(capitalize(part));
        }
    }

    let version = version_parts.join(".");
    let tier = tier_parts.join(" ");

    if tier.is_empty() {
        format!("Gemini {}", version)
    } else {
        format!("Gemini {} {}", version, tier)
    }
}

/// Parse Codex model name: codex-{variant}(-latest) → Codex {Variant}
fn parse_codex_name(rest: &str) -> String {
    let rest = rest.strip_suffix("-latest").unwrap_or(rest);
    if rest.is_empty() {
        return "Codex".to_string();
    }
    let parts: Vec<String> = rest.split('-').map(capitalize).collect();
    format!("Codex {}", parts.join(" "))
}

/// Parse OpenAI o-series: o{N}, o{N}-mini, o{N}-pro, etc.
fn parse_o_series(name: &str) -> String {
    // e.g., "o1" → "o1"
    // e.g., "o1-mini" → "o1 Mini"
    if let Some(pos) = name.find('-') {
        let base = &name[..pos];
        let suffix = &name[pos + 1..];
        format!("{} {}", base, capitalize(suffix))
    } else {
        name.to_string()
    }
}

/// Capitalize first letter of a string
fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
    }
}

/// Format version string: "4-5" → "4.5", "4" → "4"
fn format_version(version: &str) -> String {
    version.replace('-', ".")
}

/// Normalize a model name to canonical form.
///
/// Transformations:
/// - Dots to hyphens: "claude-opus-4.5" → "claude-opus-4-5"
/// - Remove date suffix: "claude-opus-4-5-20251101" → "claude-opus-4-5"
///
/// # Examples
/// ```
/// use toktrack::services::normalizer::normalize_model_name;
///
/// assert_eq!(normalize_model_name("claude-opus-4-5-20251101"), "claude-opus-4-5");
/// assert_eq!(normalize_model_name("claude-opus-4.5"), "claude-opus-4-5");
/// ```
pub fn normalize_model_name(model: &str) -> String {
    let lower = model.to_lowercase();
    if lower == "gemini-default" || lower == "gemini/default" {
        return "gemini-3-5-flash".to_string();
    }
    if lower == "gemini-3-flash-a" || lower == "gemini/gemini-3-flash-a" {
        return "gemini-3-flash-preview".to_string();
    }
    if lower == "gemini-3-pro-a" || lower == "gemini/gemini-3-pro-a" {
        return "gemini-3-pro-preview".to_string();
    }

    // Step 1: Replace dots with hyphens
    let normalized = model.replace('.', "-");

    // Step 2: Remove 8-digit date suffix at end (e.g., -20251101)
    // Pattern: ends with -YYYYMMDD where YYYYMMDD is 8 digits starting with 20
    if let Some(suffix_start) = normalized.rfind('-') {
        let suffix = &normalized[suffix_start + 1..];
        if suffix.len() == 8
            && suffix.starts_with("20")
            && suffix.chars().all(|c| c.is_ascii_digit())
        {
            return normalized[..suffix_start].to_string();
        }
    }

    normalized
}

#[cfg(test)]
mod tests {
    use super::*;

    // ========== display_name tests ==========

    #[test]
    fn test_display_name_claude_opus_4_5() {
        assert_eq!(display_name("claude-opus-4-5"), "Opus 4.5");
    }

    #[test]
    fn test_display_name_claude_sonnet_4() {
        assert_eq!(display_name("claude-sonnet-4"), "Sonnet 4");
    }

    #[test]
    fn test_display_name_claude_haiku_4_5() {
        assert_eq!(display_name("claude-haiku-4-5"), "Haiku 4.5");
    }

    #[test]
    fn test_display_name_claude_sonnet_3_5() {
        assert_eq!(display_name("claude-sonnet-3-5"), "Sonnet 3.5");
    }

    #[test]
    fn test_display_name_gpt_4o() {
        assert_eq!(display_name("gpt-4o"), "GPT-4o");
    }

    #[test]
    fn test_display_name_gpt_4o_mini() {
        assert_eq!(display_name("gpt-4o-mini"), "GPT-4o Mini");
    }

    #[test]
    fn test_display_name_gpt_4_turbo() {
        assert_eq!(display_name("gpt-4-turbo"), "GPT-4 Turbo");
    }

    #[test]
    fn test_display_name_gemini_2_5_pro() {
        assert_eq!(display_name("gemini-2-5-pro"), "Gemini 2.5 Pro");
    }

    #[test]
    fn test_display_name_gemini_2_0_flash() {
        assert_eq!(display_name("gemini-2-0-flash"), "Gemini 2.0 Flash");
    }

    #[test]
    fn test_display_name_o1() {
        assert_eq!(display_name("o1"), "o1");
    }

    #[test]
    fn test_display_name_o1_mini() {
        assert_eq!(display_name("o1-mini"), "o1 Mini");
    }

    #[test]
    fn test_display_name_o3_mini() {
        assert_eq!(display_name("o3-mini"), "o3 Mini");
    }

    #[test]
    fn test_display_name_gpt_4_1() {
        assert_eq!(display_name("gpt-4-1"), "GPT-4.1");
    }

    #[test]
    fn test_display_name_gpt_4_1_mini() {
        assert_eq!(display_name("gpt-4-1-mini"), "GPT-4.1 Mini");
    }

    #[test]
    fn test_display_name_gpt_5_2_codex() {
        assert_eq!(display_name("gpt-5-2-codex"), "GPT-5.2 Codex");
    }

    #[test]
    fn test_display_name_o4_mini() {
        assert_eq!(display_name("o4-mini"), "o4 Mini");
    }

    #[test]
    fn test_display_name_o4() {
        assert_eq!(display_name("o4"), "o4");
    }

    #[test]
    fn test_display_name_codex_mini_latest() {
        assert_eq!(display_name("codex-mini-latest"), "Codex Mini");
    }

    #[test]
    fn test_display_name_codex_mini() {
        assert_eq!(display_name("codex-mini"), "Codex Mini");
    }

    #[test]
    fn test_display_name_unknown_model() {
        assert_eq!(display_name("unknown-model"), "unknown-model");
    }

    #[test]
    fn test_display_name_empty() {
        assert_eq!(display_name(""), "");
    }

    // ========== Dot to hyphen conversion ==========

    #[test]
    fn test_dot_to_hyphen_single() {
        assert_eq!(normalize_model_name("claude-opus-4.5"), "claude-opus-4-5");
    }

    #[test]
    fn test_dot_to_hyphen_multiple() {
        assert_eq!(normalize_model_name("model-1.2.3"), "model-1-2-3");
    }

    // ========== Date suffix removal ==========

    #[test]
    fn test_remove_date_suffix_claude_opus() {
        assert_eq!(
            normalize_model_name("claude-opus-4-5-20251101"),
            "claude-opus-4-5"
        );
    }

    #[test]
    fn test_remove_date_suffix_claude_sonnet() {
        assert_eq!(
            normalize_model_name("claude-sonnet-4-20250514"),
            "claude-sonnet-4"
        );
    }

    #[test]
    fn test_remove_date_suffix_with_dot_and_date() {
        // Combined: dot + date
        assert_eq!(
            normalize_model_name("claude-opus-4.5-20251101"),
            "claude-opus-4-5"
        );
    }

    // ========== No-op cases ==========

    #[test]
    fn test_normalize_gemini_default() {
        assert_eq!(normalize_model_name("gemini-default"), "gemini-3-5-flash");
    }

    #[test]
    fn test_normalize_gemini_3_flash_a() {
        assert_eq!(
            normalize_model_name("gemini-3-flash-a"),
            "gemini-3-flash-preview"
        );
    }

    #[test]
    fn test_already_normalized() {
        assert_eq!(normalize_model_name("claude-opus-4-5"), "claude-opus-4-5");
    }

    #[test]
    fn test_no_date_suffix() {
        assert_eq!(normalize_model_name("gpt-4o"), "gpt-4o");
    }

    #[test]
    fn test_empty_string() {
        assert_eq!(normalize_model_name(""), "");
    }

    #[test]
    fn test_unknown_model() {
        assert_eq!(normalize_model_name("unknown-model"), "unknown-model");
    }

    // ========== Edge cases ==========

    #[test]
    fn test_short_date_not_removed() {
        // 8-digit number in middle shouldn't be removed
        assert_eq!(
            normalize_model_name("model-12345678-extra"),
            "model-12345678-extra"
        );
    }

    #[test]
    fn test_date_suffix_at_end_only() {
        // Date must be at end
        assert_eq!(normalize_model_name("20251101-claude"), "20251101-claude");
    }
}
