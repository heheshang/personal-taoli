//! Runs the analysis pipeline against a cache directory (used for end-to-end checks).
//!
//! Usage: `cargo run -p personal-taoli-uzi --example analyze -- <cache_dir> [quant]`

use std::path::PathBuf;

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let cache_dir = PathBuf::from(
        args.next()
            .unwrap_or_else(|| panic!("usage: analyze <cache_dir> [is_quant_factor_style]")),
    );
    let quant = args.next().map(|v| v == "true").unwrap_or(false);

    let outcome = personal_taoli_uzi::pipeline::analyze(&cache_dir, quant, None)?;
    println!("ticker          : {}", outcome.ticker);
    println!("overall_score   : {}", outcome.overall_score);
    println!("verdict_label   : {}", outcome.verdict_label);
    println!("detected_style  : {}", outcome.detected_style);
    println!("panel_consensus : {}", outcome.panel_consensus);
    println!("investors       : {}", outcome.investor_count);
    println!("wrote           : dimensions.json panel.json synthesis.json");
    Ok(())
}
