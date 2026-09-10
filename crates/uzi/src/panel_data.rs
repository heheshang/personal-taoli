//! Static tables for the panel stage, extracted from the Python source.
//!
//! These are *data*: keyword lists, supply-chain tiers, persona comment
//! templates, authored profiles, market scope, known holdings, industry
//! affinity and LHB seat ranges. `tools/compile_panel_data.py` regenerates
//! them from the original modules so they cannot drift by transcription.

use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeMap;

/// Schema this build expects from `tools/compile_panel_data.py`.
pub const PANEL_DATA_SCHEMA: i64 = 1;

/// One supply-chain layer: most-upstream match wins.
///
/// Serialised as the triple `[name, weight, [keywords]]`, matching the Python
/// literal, so it needs a tuple-shaped deserializer rather than field names.
#[derive(Debug, Clone)]
pub struct Tier {
    pub name: String,
    pub weight: f64,
    pub keywords: Vec<String>,
}

impl<'de> Deserialize<'de> for Tier {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let (name, weight, keywords): (String, f64, Vec<String>) = Deserialize::deserialize(d)?;
        Ok(Self {
            name,
            weight,
            keywords,
        })
    }
}

/// The literal tables lifted out of `extract_features`.
#[derive(Debug, Clone, Deserialize)]
pub struct FeatureTables {
    #[serde(rename = "_AI_CHOKEPOINT_KW")]
    pub ai_chokepoint_kw: Vec<String>,
    #[serde(rename = "_TIER_MAP")]
    pub tier_map: Vec<Tier>,
    #[serde(rename = "_HARD_EVIDENCE_KW")]
    pub hard_evidence_kw: Vec<String>,
    #[serde(rename = "_NO_DATA_KEYS")]
    pub no_data_keys: Vec<String>,
}

/// Metadata for one investor, from the database of record.
#[derive(Debug, Clone, Deserialize)]
pub struct InvestorMeta {
    pub name: String,
    pub group: String,
    #[serde(default = "default_mandate")]
    pub mandate: String,
}

fn default_mandate() -> String {
    "long".to_string()
}

/// One recorded holding entry. The Python tuple is
/// `(match_key, attitude, note)`, where `match_key` is matched against both
/// ticker and company name.
#[derive(Debug, Clone, Deserialize)]
pub struct Holding(pub String, pub String, pub String);

impl Holding {
    pub fn match_key(&self) -> &str {
        &self.0
    }
    pub fn attitude(&self) -> &str {
        &self.1
    }
    pub fn note(&self) -> &str {
        &self.2
    }
}

/// `INDUSTRY_AFFINITY[investor] = {"love": [...], "hate": [...], "note": "..."}`.
#[derive(Debug, Clone, Deserialize)]
pub struct Affinity {
    #[serde(default)]
    pub love: Vec<String>,
    #[serde(default)]
    pub hate: Vec<String>,
    #[serde(default)]
    pub note: String,
}

/// Everything the panel stage needs that is not logic.
#[derive(Debug, Clone, Deserialize)]
pub struct PanelData {
    pub schema: i64,
    pub personas: BTreeMap<String, BTreeMap<String, Vec<String>>>,
    pub persona_fallback: BTreeMap<String, Vec<String>>,
    pub profiles: BTreeMap<String, BTreeMap<String, String>>,
    pub group_default: BTreeMap<String, BTreeMap<String, String>>,
    pub profile_fallback: BTreeMap<String, String>,
    pub market_scope: BTreeMap<String, String>,
    pub known_holdings: BTreeMap<String, Vec<Holding>>,
    pub industry_affinity: BTreeMap<String, Affinity>,
    #[serde(default)]
    pub seats: BTreeMap<String, Value>,
    /// Database order. The panel iterates in this order so the report's judge
    /// sequence matches the original rather than an alphabetical one.
    pub investor_order: Vec<String>,
    pub investor_meta: BTreeMap<String, InvestorMeta>,
    pub feature_tables: FeatureTables,
}

impl PanelData {
    pub fn parse(json: &str) -> anyhow::Result<Self> {
        let data: PanelData = serde_json::from_str(json)
            .map_err(|e| anyhow::anyhow!("embedded panel_data.json is invalid: {e}"))?;
        if data.schema != PANEL_DATA_SCHEMA {
            anyhow::bail!(
                "embedded panel_data.json has schema {} but this build expects {}; \
                 re-run tools/compile_panel_data.py",
                data.schema,
                PANEL_DATA_SCHEMA
            );
        }
        Ok(data)
    }

    /// The 3-field authentic profile, mirroring `get_profile`'s fallback chain.
    pub fn profile(&self, investor_id: &str) -> BTreeMap<String, String> {
        if let Some(p) = self.profiles.get(investor_id) {
            return p.clone();
        }
        let group = self
            .investor_meta
            .get(investor_id)
            .map(|m| m.group.as_str())
            .unwrap_or("");
        if !group.is_empty()
            && let Some(p) = self.group_default.get(group)
        {
            return p.clone();
        }
        self.profile_fallback.clone()
    }
}
