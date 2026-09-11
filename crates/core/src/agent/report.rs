//! The structured report contract, and how a turn asks for one.
//!
//! The runtime's `turn/start` takes an optional `outputSchema`: *"JSON Schema used
//! to constrain the final assistant message for this turn."* Measured against the
//! live runtime, that is exactly what it does — the model returned
//! `{"ticker":…,"verdict":…,"score":…}` as the whole message, no surrounding
//! prose, while its earlier progress messages stayed ordinary text.
//!
//! Two consequences shape this module:
//!
//! * **Only the final message is constrained.** Progress reporting is unaffected,
//!   so a long analysis still streams its intermediate messages as prose and then
//!   closes with the structured result.
//! * **The schema is a repository document, not a string in code.** It is a
//!   contract between three parties — the model that must satisfy it, the loader
//!   that sends it, and the interface that renders it — and the first two should
//!   not be able to drift from the third. Keeping it versioned beside the domain
//!   instructions means a change is reviewed like any other contract change.
//!
//! A missing or malformed schema is a **failure**, for the same reason missing
//! instructions are: a turn that silently ran unconstrained would look like it
//! worked and produce prose where the interface expects a report.

use std::path::{Path, PathBuf};

use serde_json::Value;

/// Repository-relative location of the report schema.
pub const DEFAULT_PATH: &str = "docs/agent/report-schema.json";

/// Why the schema could not be used.
#[derive(Debug)]
pub enum ReportSchemaError {
    /// Not found, including after searching parent directories.
    NotFound { searched: Vec<PathBuf> },
    /// Present but unreadable.
    Unreadable {
        path: PathBuf,
        source: std::io::Error,
    },
    /// Present but not a JSON object, or missing the properties a report needs.
    Invalid { path: PathBuf, detail: String },
}

impl std::fmt::Display for ReportSchemaError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound { searched } => write!(
                formatter,
                "找不到报告结构定义 {DEFAULT_PATH}（已查找：{}）",
                searched
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Self::Unreadable { path, source } => {
                write!(
                    formatter,
                    "无法读取报告结构定义 {}：{source}",
                    path.display()
                )
            }
            Self::Invalid { path, detail } => {
                write!(
                    formatter,
                    "报告结构定义 {} 不合法：{detail}",
                    path.display()
                )
            }
        }
    }
}

impl std::error::Error for ReportSchemaError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Unreadable { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// Resolves [`DEFAULT_PATH`] from `base`, then each ancestor.
fn resolve_from(base: &Path) -> Option<PathBuf> {
    let relative = Path::new(DEFAULT_PATH);
    let direct = base.join(relative);
    if direct.is_file() {
        return Some(direct);
    }
    base.ancestors()
        .map(|ancestor| ancestor.join(relative))
        .find(|candidate| candidate.is_file())
}

/// Resolves [`DEFAULT_PATH`] from the working directory, then the executable's
/// ancestors — the same search the instructions loader uses, so both documents
/// resolve together or not at all.
pub fn resolve_default() -> Option<PathBuf> {
    if let Ok(cwd) = std::env::current_dir()
        && let Some(found) = resolve_from(&cwd)
    {
        return Some(found);
    }
    let executable = std::env::current_exe().ok()?;
    resolve_from(executable.parent()?)
}

/// Reads and validates the schema at `path`.
///
/// Validation is shallow on purpose: it checks that the document is a JSON object
/// with a `properties` map containing the three keys the interface needs to render
/// a report at all. Deeper validation belongs to the runtime, which rejects a
/// schema it cannot enforce — duplicating a JSON Schema validator here would be a
/// second implementation to keep in step.
pub fn load(path: &Path) -> Result<Value, ReportSchemaError> {
    let contents = std::fs::read_to_string(path).map_err(|source| {
        if source.kind() == std::io::ErrorKind::NotFound {
            ReportSchemaError::NotFound {
                searched: vec![path.to_path_buf()],
            }
        } else {
            ReportSchemaError::Unreadable {
                path: path.to_path_buf(),
                source,
            }
        }
    })?;

    let value: Value =
        serde_json::from_str(&contents).map_err(|error| ReportSchemaError::Invalid {
            path: path.to_path_buf(),
            detail: error.to_string(),
        })?;

    let properties = value
        .get("properties")
        .and_then(Value::as_object)
        .ok_or_else(|| ReportSchemaError::Invalid {
            path: path.to_path_buf(),
            detail: "缺少 `properties` 对象".to_string(),
        })?;

    for key in REQUIRED_PROPERTIES {
        if !properties.contains_key(key) {
            return Err(ReportSchemaError::Invalid {
                path: path.to_path_buf(),
                detail: format!("缺少界面渲染所需的属性 `{key}`"),
            });
        }
    }
    Ok(value)
}

/// Properties the interface cannot render a report without.
///
/// These mirror the schema's own `required` list: the renderer keys its header and
/// summary on them, so a schema that stopped emitting one would produce a report
/// the interface could not show.
pub const REQUIRED_PROPERTIES: [&str; 3] = ["ticker", "name", "summary"];

/// Resolves and loads [`DEFAULT_PATH`].
pub fn load_default() -> Result<Value, ReportSchemaError> {
    let path = resolve_default().ok_or_else(|| ReportSchemaError::NotFound {
        searched: vec![PathBuf::from(DEFAULT_PATH)],
    })?;
    load(&path)
}

/// The instruction fragment that tells the model what this turn must produce.
///
/// Needed because the schema alone was measured **not** to be enough: without a
/// stated requirement the model refused, saying no structure had been provided. The
/// fragment is built from the schema rather than written out separately, so the
/// field names it names are the schema's own and cannot drift from it.
///
/// Returns `(source, value)`; the runtime keys `additionalContext` by source, and a
/// stable one lets a later reader tell where the context came from.
pub fn context_fragment(schema: &Value) -> (String, String) {
    let join = |keys: Vec<String>| keys.join("、");
    let required = join(
        schema
            .get("required")
            .and_then(Value::as_array)
            .map(|keys| {
                keys.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default(),
    );
    let sections = join(
        schema
            .get("properties")
            .and_then(Value::as_object)
            .map(|properties| {
                properties
                    .keys()
                    .filter(|key| !REQUIRED_PROPERTIES.contains(&key.as_str()))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default(),
    );

    let value = format!(
        "本轮**最终**回复必须是一个 JSON 对象，且直接使用下列字段名：\n\
         - 必须有：{required}\n\
         - 可选分节：{sections}\n\
         整条回复就是这一个 JSON：不要用 ``` 代码围栏包裹，不要在前后添加任何说明文字。\n\
         本轮要求了结构化输出，因此领域指令中「正文必须出现在回复里」由 `summary` 字段承载；\n\
         该字段内部可以使用 Markdown，但回复整体仍是 JSON。\n\
         不要自创字段名；取不到的数据请省略该字段或留空，不要编造。"
    );
    ("taoli://report-schema".to_string(), value)
}

/// The structure a report-requesting turn produced, and whether it conformed.
///
/// One object rather than a value plus a separate "conformed" flag, because those
/// two can disagree: a flag with no value would claim a report exists, and a value
/// with no flag would leave the renderer guessing. Here the only states are "no
/// report" (`None`) and "this report, conforming or not".
///
/// A **non-conforming** report is kept rather than dropped, and the interface shows
/// it as raw metadata. Measured, a model that ignores the schema still returns
/// *something* structured — with its own field names — and a turn that took minutes
/// to produce it should not have that thrown away because three key names differ.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnReport {
    /// The object the model returned, exactly as received.
    pub value: Value,
    /// Whether it satisfied the schema. `false` means the field names or shapes are
    /// the model's own invention and must not be rendered as a report.
    pub conforms: bool,
}

/// Checks that `value` is a report the interface can render.
///
/// Validation is shallow by design — an object carrying the three properties the
/// interface keys on, each present and not null. Deeper checking would duplicate the
/// runtime's schema engine for no gain; what this catches is the case that actually
/// happens, measured: a model returning JSON with **its own** field names. That
/// parses fine and renders as a blank report, which is worse than falling back to
/// the prose.
pub fn validate(value: &Value) -> Result<(), String> {
    let Some(object) = value.as_object() else {
        return Err("输出不是 JSON 对象".to_string());
    };
    let missing: Vec<&str> = REQUIRED_PROPERTIES
        .into_iter()
        .filter(|key| object.get(*key).is_none_or(Value::is_null))
        .collect();
    if missing.is_empty() {
        return Ok(());
    }
    Err(format!(
        "输出缺少必需字段 {}（实际字段：{}）",
        missing.join("、"),
        object.keys().cloned().collect::<Vec<_>>().join("、")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("taoli-schema-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create scratch");
        dir
    }

    #[test]
    fn the_repository_schema_loads_and_carries_what_the_interface_needs() {
        let Ok(schema) = load_default() else {
            eprintln!("skipping: {DEFAULT_PATH} is absent");
            return;
        };
        let properties = schema["properties"].as_object().expect("properties");
        for key in REQUIRED_PROPERTIES {
            assert!(properties.contains_key(key), "missing {key}");
        }
        // The sections the report renders. Losing one silently would show a report
        // with a blank panel, so they are pinned here.
        for key in [
            "quote",
            "financial",
            "technical",
            "valuation",
            "events",
            "integrity",
        ] {
            assert!(properties.contains_key(key), "missing section {key}");
        }
        // A closed schema: the runtime enforces `additionalProperties`, and an
        // open object would let the model invent fields the interface ignores.
        assert_eq!(schema["additionalProperties"], serde_json::json!(false));
    }

    #[test]
    fn every_object_in_the_schema_is_closed() {
        // Pinned because an open nested object is how a contract quietly becomes
        // advisory: the model may add fields, and no one notices they are ignored.
        fn walk(node: &Value, path: &str) {
            if let Some(object) = node.as_object() {
                if object.get("type").and_then(Value::as_str) == Some("object")
                    || object.contains_key("properties")
                {
                    assert_eq!(
                        object.get("additionalProperties"),
                        Some(&Value::Bool(false)),
                        "object at {path} is not closed"
                    );
                }
                for (key, child) in object {
                    walk(child, &format!("{path}.{key}"));
                }
            } else if let Some(items) = node.as_array() {
                for (index, child) in items.iter().enumerate() {
                    walk(child, &format!("{path}[{index}]"));
                }
            }
        }
        let Ok(schema) = load_default() else {
            eprintln!("skipping: {DEFAULT_PATH} is absent");
            return;
        };
        walk(&schema, "$");
    }

    #[test]
    fn a_schema_missing_a_required_property_is_refused() {
        // The failure this guards: a report the interface cannot render, produced
        // by a turn that believed it succeeded.
        let dir = scratch("missing-prop");
        let path = dir.join("schema.json");
        std::fs::write(
            &path,
            r#"{"type":"object","properties":{"ticker":{"type":"string"}}}"#,
        )
        .expect("write");
        let error = load(&path).expect_err("must refuse");
        match &error {
            ReportSchemaError::Invalid { detail, .. } => {
                assert!(
                    detail.contains("name") || detail.contains("summary"),
                    "{detail}"
                );
            }
            other => panic!("expected Invalid, got {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn malformed_json_is_refused_rather_than_sent() {
        let dir = scratch("malformed");
        let path = dir.join("schema.json");
        std::fs::write(&path, "{ not json").expect("write");
        assert!(matches!(
            load(&path).expect_err("must refuse"),
            ReportSchemaError::Invalid { .. }
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_file_names_the_expected_path() {
        let error = load(Path::new("/nonexistent/report-schema.json")).expect_err("must fail");
        assert!(matches!(error, ReportSchemaError::NotFound { .. }));
        assert!(error.to_string().contains(DEFAULT_PATH), "{error}");
    }

    #[test]
    fn resolution_walks_up_to_the_document() {
        let dir = scratch("ancestors");
        let nested = dir.join("a").join("b");
        std::fs::create_dir_all(&nested).expect("mkdir");
        std::fs::create_dir_all(dir.join("docs").join("agent")).expect("mkdir docs");
        std::fs::write(
            dir.join(DEFAULT_PATH),
            r#"{"type":"object","properties":{"ticker":{},"name":{},"summary":{}}}"#,
        )
        .expect("write");
        assert_eq!(resolve_from(&nested), Some(dir.join(DEFAULT_PATH)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_context_fragment_names_the_schemas_own_fields() {
        let schema = load_default().expect("schema present");
        let (source, value) = context_fragment(&schema);
        assert_eq!(source, "taoli://report-schema");
        // The required keys must appear verbatim, or the fragment cannot fix the
        // problem it exists for: the model not knowing the field names — measured,
        // it otherwise invented its own.
        for key in REQUIRED_PROPERTIES {
            assert!(value.contains(key), "fragment omits {key}: {value}");
        }
        // And it must forbid the two behaviours the measurements showed.
        assert!(value.contains("不要自创字段名"), "{value}");
        assert!(value.contains("不要编造"), "{value}");
    }

    #[test]
    fn validation_accepts_a_conforming_report() {
        assert!(
            validate(&serde_json::json!({
                "ticker": "002600.SZ", "name": "领益智造", "summary": "……"
            }))
            .is_ok()
        );
    }

    /// The shape the model actually produced when the schema was not conveyed
    /// forcefully enough: JSON, plausible, and useless to the renderer.
    #[test]
    fn validation_rejects_invented_field_names_and_says_what_arrived() {
        let error = validate(&serde_json::json!({
            "stock_code": "002600.SZ",
            "company_name": "领益智造",
            "业务结构_2026H1": []
        }))
        .expect_err("invented fields must be refused");
        assert!(error.contains("ticker"), "{error}");
        // The message must name what did arrive, or the failure is undiagnosable.
        assert!(error.contains("stock_code"), "{error}");
    }

    #[test]
    fn a_null_required_field_counts_as_missing() {
        let error = validate(&serde_json::json!({
            "ticker": "002600.SZ", "name": null, "summary": "……"
        }))
        .expect_err("a null name is not a name");
        assert!(error.contains("name"), "{error}");
    }

    #[test]
    fn a_non_object_is_refused() {
        assert!(validate(&serde_json::json!("just text")).is_err());
        assert!(validate(&serde_json::json!([1, 2])).is_err());
    }
}
