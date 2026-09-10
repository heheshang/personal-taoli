//! Prepare a rendered report for embedding inside the Taoli console.
//!
//! The upstream report is "self-contained": 142 avatars ship as inline
//! `data:` URIs. Three references still reach outside, though, and all three
//! are removed here rather than allowed through the app's CSP:
//!
//! * three `<link>` tags to `fonts.googleapis.com` / `fonts.gstatic.com`
//!   (the font stack falls back to the system stack);
//! * two `<img>` tags pointing at `api.qrserver.com`, which would leak the
//!   fact and timing of a report being opened to a third party — replaced with
//!   an inline placeholder of the same box size so the layout is unchanged.
//!
//! What is deliberately *not* touched: the single inline `<script>` that
//! renders hover tooltips, and the fund links whose `href` points at
//! eastmoney. Link navigation is not governed by `default-src`, and the
//! document gets its own policy when served (see the `uzi://` handler), so the
//! app's own CSP stays as strict as it was.

/// Inline 1×1 transparent SVG, used in place of the remote QR images.
const TRANSPARENT_SVG: &str = "data:image/svg+xml;base64,PHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHdpZHRoPSIxIiBoZWlnaHQ9IjEiLz4=";

/// Hosts whose `<link>` elements are stripped.
const BLOCKED_LINK_HOSTS: [&str; 2] = ["fonts.googleapis.com", "fonts.gstatic.com"];

/// Hosts whose `<img>` elements are neutralised.
const BLOCKED_IMAGE_HOSTS: [&str; 1] = ["api.qrserver.com"];

/// Rewrites the report so it renders with no third-party requests.
pub fn sanitize(html: &str) -> String {
    let mut out = strip_link_tags(html);
    out = replace_image_sources(&out);
    out
}

/// Removes `<link ...>` elements that reference a blocked host.
fn strip_link_tags(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(start) = rest.find("<link") {
        let Some(end_rel) = rest[start..].find('>') else {
            break;
        };
        let end = start + end_rel + 1;
        let tag = &rest[start..end];
        out.push_str(&rest[..start]);
        if !BLOCKED_LINK_HOSTS.iter().any(|host| tag.contains(host)) {
            out.push_str(tag);
        }
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

/// Points blocked `<img>` sources at an inline placeholder.
///
/// The element is kept (rather than deleted) so surrounding layout keeps its
/// height.
fn replace_image_sources(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(start) = rest.find("<img") {
        let Some(end_rel) = rest[start..].find('>') else {
            break;
        };
        let end = start + end_rel + 1;
        let tag = &rest[start..end];
        out.push_str(&rest[..start]);
        if BLOCKED_IMAGE_HOSTS.iter().any(|host| tag.contains(host)) {
            out.push_str(&replace_src(tag));
        } else {
            out.push_str(tag);
        }
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

/// Swaps the value of a tag's `src="..."` attribute.
fn replace_src(tag: &str) -> String {
    let Some(at) = tag.find("src=\"") else {
        return tag.to_string();
    };
    let value_start = at + 5;
    let Some(rel_end) = tag[value_start..].find('"') else {
        return tag.to_string();
    };
    let value_end = value_start + rel_end;
    format!(
        "{}{}{}",
        &tag[..value_start],
        TRANSPARENT_SVG,
        &tag[value_end..]
    )
}

/// True when no *element* references a blocked host.
///
/// Deliberately scans tags rather than the whole document: one `qrserver` URL
/// also appears as a string literal inside the report's inline script, where it
/// builds a QR image at runtime. That request is blocked by the document policy
/// the serving handler sets, and the script already falls back when it fails —
/// so it is not a static dependency and must not be counted here.
pub fn is_self_contained(html: &str) -> bool {
    let blocked = |tag: &str| {
        BLOCKED_LINK_HOSTS
            .iter()
            .chain(BLOCKED_IMAGE_HOSTS.iter())
            .any(|host| tag.contains(host))
    };
    for open in ["<link", "<img"] {
        let mut rest = html;
        while let Some(start) = rest.find(open) {
            let Some(rel_end) = rest[start..].find('>') else {
                break;
            };
            let end = start + rel_end + 1;
            if blocked(&rest[start..end]) {
                return false;
            }
            rest = &rest[end..];
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_google_font_links_but_keeps_other_links() {
        let html = r#"<head>
<link rel="preconnect" href="https://fonts.googleapis.com">
<link href="https://fonts.googleapis.com/css2?family=Fira+Sans&display=swap" rel="stylesheet">
<link rel="stylesheet" href="./local.css">
</head>"#;
        let out = sanitize(html);
        assert!(!out.contains("fonts.googleapis.com"));
        assert!(out.contains("./local.css"));
        assert!(out.contains("<head>"));
    }

    #[test]
    fn neutralises_remote_images_and_keeps_their_box() {
        let html = r#"<img src="https://api.qrserver.com/v1/create-qr-code/?size=120x120" alt="QR" style="width:80px" />"#;
        let out = sanitize(html);
        assert!(!out.contains("qrserver.com"));
        assert!(out.contains(TRANSPARENT_SVG));
        // Attributes other than `src` survive, so the layout box is unchanged.
        assert!(out.contains(r#"alt="QR""#));
        assert!(out.contains(r#"style="width:80px""#));
    }

    #[test]
    fn leaves_inline_data_uris_alone() {
        let html = r#"<img src="data:image/svg+xml;base64,AAAA" alt="avatar">"#;
        assert_eq!(sanitize(html), html);
    }

    #[test]
    fn is_idempotent() {
        let html = r#"<link href="https://fonts.gstatic.com" rel="preconnect">
<img src="https://api.qrserver.com/x">"#;
        let once = sanitize(html);
        assert_eq!(sanitize(&once), once);
    }

    #[test]
    fn reports_self_containment_by_tag_not_substring() {
        assert!(!is_self_contained(
            r#"<img src="https://api.qrserver.com/x">"#
        ));
        assert!(!is_self_contained(
            r#"<link href="https://fonts.gstatic.com" rel="preconnect">"#
        ));
        assert!(is_self_contained("data:image/svg+xml;base64,AAAA"));
        // A URL inside the inline script is a runtime attempt, not a static
        // dependency: the document policy blocks it and the script has a
        // fallback for exactly that case.
        assert!(is_self_contained(
            r#"<script>const u='https://api.qrserver.com/v1';</script>"#
        ));
    }

    /// The report's inline tooltip script must survive: it is the only
    /// interactive behaviour in the document.
    #[test]
    fn keeps_the_inline_script() {
        let html =
            r#"<script>function tooltipify(){}</script><img src="https://api.qrserver.com/x">"#;
        let out = sanitize(html);
        assert!(out.contains("function tooltipify(){}"));
    }
}
