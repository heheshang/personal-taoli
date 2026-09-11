//! Minimal JSON-RPC 2.0 envelope plus the codex `app-server` handshake types.
//!
//! Only the subset required by PORT-01 is modelled. Field names and shapes are
//! taken from the schema the binary itself emits
//! (`codex app-server generate-json-schema [--experimental]`), not from prose
//! documentation, so that a protocol drift shows up as a parse failure here
//! rather than as a silent mismatch.
//!
//! Framing is newline-delimited JSON: the transport reads stdin line by line
//! (`app-server-transport/src/transport/stdio.rs`) and appends `\n` to every
//! outgoing message (`.../stdio.rs`, `json.push('\n')`).

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Request identifier. The schema types this as `string | int64`; this client
/// only ever emits integers, which the server accepts.
pub type RequestId = i64;

/// JSON-RPC error code for "this client will not handle that request".
///
/// Used when the server asks for something outside the implemented surface
/// (for example an approval flow in a round that has not built one yet).
/// Replying with an error is deliberate: it fails the operation closed instead
/// of fabricating a decision.
pub const METHOD_NOT_IMPLEMENTED: i64 = -32601;

/// A client → server request. `params` is always present here: every method
/// this client calls requires parameters.
#[derive(Debug, Clone, Serialize)]
pub struct RpcRequest<'a> {
    pub id: RequestId,
    pub method: &'a str,
    pub params: Value,
}

/// A response to a server → client request.
#[derive(Debug, Clone, Serialize)]
pub struct RpcResponse {
    pub id: RequestId,
    pub result: Value,
}

/// A failed response to a server → client request.
#[derive(Debug, Clone, Serialize)]
pub struct RpcError {
    pub id: RequestId,
    pub error: RpcErrorBody,
}

#[derive(Debug, Clone, Serialize)]
pub struct RpcErrorBody {
    pub code: i64,
    pub message: String,
}

impl RpcError {
    pub fn new(id: RequestId, code: i64, message: impl Into<String>) -> Self {
        Self {
            id,
            error: RpcErrorBody {
                code,
                message: message.into(),
            },
        }
    }
}

/// Classification of one inbound line.
///
/// Order matters and is not obvious: a **server → client request** carries both
/// `id` and `method`, so `method` must be tested before `id` — otherwise every
/// inbound request is misread as a response, as the initial implementation did
/// before it was exercised against the live runtime.
#[derive(Debug)]
pub enum Incoming<'a> {
    /// `{id, method, params}` — the server asks this client to do something.
    Request {
        id: RequestId,
        method: &'a str,
        params: Option<&'a Value>,
    },
    /// `{id, result}` — a successful response to one of our requests.
    Response { id: RequestId, result: &'a Value },
    /// `{id, error}` — a failed response to one of our requests.
    Error { id: RequestId, error: &'a Value },
    /// `{method, params?}` — a server-initiated message we only observe.
    Notification {
        method: &'a str,
        params: Option<&'a Value>,
    },
}

/// Classifies one decoded JSON-RPC message.
///
/// Returns `None` when the object matches none of the recognised shapes, so
/// that the caller can report the offending payload verbatim instead of
/// guessing.
pub fn classify(message: &Value) -> Option<Incoming<'_>> {
    let object: &Map<String, Value> = message.as_object()?;

    if let Some(method) = object.get("method").and_then(Value::as_str) {
        let params = object.get("params");
        return match object.get("id") {
            Some(id) => Some(Incoming::Request {
                id: id.as_i64()?,
                method,
                params,
            }),
            None => Some(Incoming::Notification { method, params }),
        };
    }

    let id = object.get("id")?.as_i64()?;
    if let Some(error) = object.get("error") {
        return Some(Incoming::Error { id, error });
    }
    object
        .get("result")
        .map(|result| Incoming::Response { id, result })
}

/// Parameters for the `initialize` handshake.
///
/// `experimentalApi` is declared because the runtime gates the fields this
/// client depends on behind it. Measured, not assumed — without the capability
/// the server answers:
///
/// ```text
/// {"code":-32600,"message":"thread/start.dynamicTools requires experimentalApi capability"}
/// ```
///
/// Schema: `InitializeParams` requires `clientInfo`; `InitializeCapabilities`
/// defaults `experimentalApi` to `false`.
pub fn initialize_params(name: &str, title: &str, version: &str) -> Value {
    serde_json::json!({
        "clientInfo": { "name": name, "title": title, "version": version },
        "capabilities": { "experimentalApi": true }
    })
}

/// When the runtime asks the host before running a command.
///
/// This is **independent of the sandbox**: a read-only sandbox still permits
/// harmless commands (the model can run `echo` without writing anything), so it
/// produces no approval traffic. Making every side-effecting action pass a human
/// gate therefore requires this policy, not just a narrow sandbox.
///
/// Measured: with the default (`OnRequest`) a read-only thread ran
/// `echo hello` with no approval request at all; with [`Self::UnlessTrusted`]
/// the runtime raised `item/commandExecution/requestApproval`.
///
/// Serialisation (`unless_trusted` / `on_request` / `never`) is the trace's
/// spelling of the *policy*; it is deliberately distinct from the wire spelling
/// produced by [`Self::as_wire`], so a value read from a trace cannot be
/// mistaken for something to put on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalPolicy {
    /// Every command requires approval unless an execpolicy rule allows it.
    ///
    /// The value a host wanting a real gate must use.
    UnlessTrusted,
    /// The model decides when to ask. The runtime's default.
    OnRequest,
    /// Never ask; failures go straight back to the model.
    Never,
}

impl ApprovalPolicy {
    /// The wire value accepted by `thread/start.approvalPolicy`.
    pub fn as_wire(self) -> &'static str {
        match self {
            Self::UnlessTrusted => "untrusted",
            Self::OnRequest => "on-request",
            Self::Never => "never",
        }
    }
}

/// Parameters for `thread/start`.
///
/// `dynamic_tools` uses the `DynamicToolSpec` wire shape; see
/// [`super::tools::DynamicToolSpec`] for the construction.
/// Parameters for `thread/start`.
///
/// `dynamic_tools` uses the `DynamicToolSpec` wire shape; see
/// [`super::tools::DynamicToolSpec`] for the construction.
///
/// `developer_instructions` carries the repository's domain constraints. The
/// runtime renders that field as an **additional fragment** rather than
/// replacing its own base instructions, which is why the project's rules go
/// here and not in `baseInstructions` — measured: the field needs no
/// experimental capability, and the model honours it.
pub fn thread_start_params(
    cwd: &str,
    ephemeral: bool,
    sandbox: &str,
    approval_policy: ApprovalPolicy,
    developer_instructions: Option<&str>,
    dynamic_tools: Vec<Value>,
) -> Value {
    let mut params = serde_json::json!({
        "cwd": cwd,
        "ephemeral": ephemeral,
        "sandbox": sandbox,
        "approvalPolicy": approval_policy.as_wire(),
        "dynamicTools": dynamic_tools,
    });
    if let Some(instructions) = developer_instructions.filter(|text| !text.trim().is_empty()) {
        params["developerInstructions"] = Value::String(instructions.to_string());
    }
    params
}

/// Parameters for `turn/start` carrying a single text input.
pub fn turn_start_params(thread_id: &str, text: &str) -> Value {
    serde_json::json!({
        "threadId": thread_id,
        "input": [{ "type": "text", "text": text }],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn classify_reads_server_requests_before_responses() {
        // The live runtime sends `{id, method, params}` for `item/tool/call`.
        // With `id` tested first this would be misread as a response and the
        // server would wait forever for a reply.
        let request = json!({
            "id": 42,
            "method": "item/tool/call",
            "params": { "tool": "taoli_shadow_report", "callId": "c1" }
        });
        match classify(&request) {
            Some(Incoming::Request { id, method, params }) => {
                assert_eq!(id, 42);
                assert_eq!(method, "item/tool/call");
                assert_eq!(
                    params.expect("params present")["tool"],
                    json!("taoli_shadow_report")
                );
            }
            other => panic!("expected Request, got {other:?}"),
        }
    }

    #[test]
    fn classify_distinguishes_null_result_from_notification() {
        // `result: null` is a valid success and must not be read as a
        // notification merely because the payload is empty.
        let success = json!({ "id": 1, "result": null });
        assert!(matches!(
            classify(&success),
            Some(Incoming::Response { id: 1, .. })
        ));

        let notification = json!({ "method": "turn/started", "params": {} });
        assert!(matches!(
            classify(&notification),
            Some(Incoming::Notification {
                method: "turn/started",
                ..
            })
        ));
    }

    #[test]
    fn classify_reads_error_responses() {
        let failure = json!({ "id": 7, "error": { "code": -32601, "message": "no such method" } });
        match classify(&failure) {
            Some(Incoming::Error { id, error }) => {
                assert_eq!(id, 7);
                assert_eq!(error["code"], json!(-32601));
            }
            other => panic!("expected an error response, got {other:?}"),
        }
    }

    #[test]
    fn classify_rejects_unrecognised_shapes() {
        // Neither `id` nor `method`: not a message this client understands.
        assert!(classify(&json!({ "params": {} })).is_none());
    }

    #[test]
    fn request_serialises_with_required_fields_only() {
        let request = RpcRequest {
            id: 3,
            method: "initialize",
            params: initialize_params("taoli", "Taoli", "0.1.0"),
        };
        let encoded = serde_json::to_value(&request).expect("serialises");
        assert_eq!(encoded["id"], json!(3));
        assert_eq!(encoded["method"], json!("initialize"));
        assert_eq!(encoded["params"]["clientInfo"]["name"], json!("taoli"));
        // Without this the runtime refuses `thread/start.dynamicTools`.
        assert_eq!(
            encoded["params"]["capabilities"]["experimentalApi"],
            json!(true)
        );
    }

    #[test]
    fn thread_start_params_carry_dynamic_tools_and_the_approval_policy() {
        let params = thread_start_params(
            "",
            true,
            "read-only",
            ApprovalPolicy::UnlessTrusted,
            Some("domain rules"),
            vec![json!({"name": "t"})],
        );
        assert_eq!(params["dynamicTools"][0]["name"], json!("t"));
        assert_eq!(params["ephemeral"], json!(true));
        assert_eq!(params["sandbox"], json!("read-only"));
        // Without this the runtime may run commands without ever asking.
        assert_eq!(params["approvalPolicy"], json!("untrusted"));
        assert_eq!(params["developerInstructions"], json!("domain rules"));
    }

    #[test]
    fn absent_or_blank_instructions_are_omitted_rather_than_sent_empty() {
        // Sending an empty string would be indistinguishable, to the runtime,
        // from having no instructions at all — and the empty case is what the
        // caller is supposed to have refused before getting here.
        for instructions in [None, Some(""), Some("   \n")] {
            let params = thread_start_params(
                "",
                true,
                "read-only",
                ApprovalPolicy::UnlessTrusted,
                instructions,
                Vec::new(),
            );
            assert!(
                params.get("developerInstructions").is_none(),
                "blank instructions must be omitted: {params}"
            );
        }
    }

    #[test]
    fn approval_policy_maps_to_the_wire_values_the_runtime_accepts() {
        // These three are the string variants of the runtime's `AskForApproval`.
        assert_eq!(ApprovalPolicy::UnlessTrusted.as_wire(), "untrusted");
        assert_eq!(ApprovalPolicy::OnRequest.as_wire(), "on-request");
        assert_eq!(ApprovalPolicy::Never.as_wire(), "never");
    }

    #[test]
    fn rpc_error_carries_a_code_and_message() {
        let encoded = serde_json::to_value(RpcError::new(9, METHOD_NOT_IMPLEMENTED, "nope"))
            .expect("serialises");
        assert_eq!(encoded["id"], json!(9));
        assert_eq!(encoded["error"]["code"], json!(METHOD_NOT_IMPLEMENTED));
        assert_eq!(encoded["error"]["message"], json!("nope"));
    }
}
