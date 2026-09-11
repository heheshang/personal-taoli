# Provenance of the macOS seatbelt sandbox

This directory contains code and policy text derived from **openai/codex**
(`codex-rs/sandboxing`), which is licensed under the **Apache License 2.0**.

- Upstream: https://github.com/openai/codex
- Path: `codex-rs/sandboxing/src/`
- Retrieved: 2026-09-10, branch `main`
- Upstream licence: Apache-2.0 (see the upstream `LICENSE` file)

## What is verbatim

`policies/*.sbpl` are **byte-identical copies** of the upstream files:

| Local file | Upstream file |
| --- | --- |
| `policies/seatbelt_base_policy.sbpl` | `seatbelt_base_policy.sbpl` |
| `policies/seatbelt_network_policy.sbpl` | `seatbelt_network_policy.sbpl` |
| `policies/seatbelt_preferences_policy.sbpl` | `seatbelt_preferences_policy.sbpl` |
| `policies/seatbelt_read_only_platform_defaults.sbpl` | `seatbelt_read_only_platform_defaults.sbpl` |

These are Chrome-derived Seatbelt policy texts. They are copied rather than
rewritten on purpose: they encode a closed-by-default policy whose gaps are not
obvious, and hand-rolling an equivalent is how sandboxes acquire holes.

Verification that they remain byte-identical:

```sh
diff <(curl -sL https://raw.githubusercontent.com/openai/codex/main/codex-rs/sandboxing/src/seatbelt_base_policy.sbpl) \
     crates/core/src/agent/sandbox/policies/seatbelt_base_policy.sbpl
```

## What is adapted

`seatbelt.rs` in this directory ports the **filesystem** half of upstream's
`seatbelt.rs`: writable-root normalisation, the access-policy builder (including
the root-anchor denies), the platform-default sections, and the `-D`/`-p`
argument assembly.

Deliberately **not** ported, with the reason:

| Omitted | Why |
| --- | --- |
| Network-proxy policy generation (`proxy_policy_inputs`, `unix_socket_policy`, `dynamic_network_policy`) | Upstream uses these to build per-command policies for its managed MITM proxy. This project confines one fixed child process and does not run that proxy; porting it would pull in `codex_network_proxy` (~28k lines) for a feature with no caller here. |
| `codex_protocol::permissions` (`FileSystemSandboxPolicy`, `SandboxPolicy`, `WritableRoot`) | Upstream's per-command permission model. The confinement needed here is a single fixed profile, so the policy is expressed as explicit read/write roots instead. |
| `PROTECTED_METADATA_PATH_NAMES` / unreadable-glob policy | Upstream protects e.g. `.git` from writes *inside a writable workspace*. This project grants no workspace write access at all, so the protection has nothing to guard. |
| `MacosSeatbeltProfile::FileSystemHelper` | Upstream uses it for its own filesystem helper processes, which this project does not run. |
| `codex-utils-absolute-path` crate | Upstream's 767-line path type. Only "is absolute and normalised" is needed here, so a small equivalent lives in `seatbelt.rs`. |

The port is Apache-2.0 code: it must keep this attribution, and any file copied
from upstream must not be relicensed.
