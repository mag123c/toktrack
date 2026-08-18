//! Regenerates the vendored offline pricing snapshot from LiteLLM.
//!
//! Deserializing into `ModelPricing` is the trim: serde drops every field the
//! struct does not declare, so the snapshot can never fall behind the fields the
//! pricing service actually reads. Run via
//! `cargo run --features snapshot-tool --bin gen_pricing_snapshot`.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use toktrack::services::pricing::ModelPricing;

const LITELLM_PRICING_URL: &str =
    "https://raw.githubusercontent.com/BerriAI/litellm/main/model_prices_and_context_window.json";

const REQUEST_TIMEOUT_SECS: u64 = 60;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("assets")
        .join("pricing_snapshot.json");

    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
        .build()?;

    let fetched: BTreeMap<String, serde_json::Value> = client
        .get(LITELLM_PRICING_URL)
        .send()?
        .error_for_status()?
        .json()?;

    let mut snapshot: BTreeMap<String, ModelPricing> = BTreeMap::new();
    let mut skipped = 0usize;
    for (model, raw) in fetched {
        // `sample_spec` and friends are documentation entries, not models.
        if !raw.is_object() {
            continue;
        }
        // One malformed upstream entry must not block the whole refresh.
        let pricing: ModelPricing = match serde_json::from_value(raw) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("[gen_pricing_snapshot] skipping {model}: {e}");
                skipped += 1;
                continue;
            }
        };
        if pricing.has_any_pricing() {
            snapshot.insert(model, pricing);
        }
    }

    if snapshot.is_empty() {
        return Err(
            "LiteLLM returned no priced models; refusing to write an empty snapshot".into(),
        );
    }

    let mut json = serde_json::to_string_pretty(&snapshot)?;
    json.push('\n');
    std::fs::write(&out_path, json)?;

    println!(
        "wrote {} priced models to {} ({} skipped)",
        snapshot.len(),
        out_path.display(),
        skipped
    );
    Ok(())
}
