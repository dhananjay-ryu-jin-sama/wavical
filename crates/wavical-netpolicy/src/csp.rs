//! Content-Security-Policy (CSP) generation for `open`, `safe`, and `ultra-safe` modes.

use thiserror::Error;

use crate::NetworkMode;
use crate::allowlist::Allowlist;
use crate::html::CspHash;
use crate::limits::MAX_INLINE_SCRIPTS_HASHED_PER_PAGE;
use crate::origin::{AllowlistPattern, ParsedOrigin};

/// Exact Phase 0 CSP string preserved verbatim for `ultra-safe` mode (Plan 1.13, Rule 4).
pub const ULTRA_SAFE_CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:; font-src 'self'; connect-src 'self'; frame-src 'none'; object-src 'none'; base-uri 'none'; frame-ancestors 'none'; form-action 'self';";

/// Errors during CSP generation.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum CspError {
    #[error("page has {0} inline scripts, exceeding limit of {MAX_INLINE_SCRIPTS_HASHED_PER_PAGE}")]
    TooManyInlineScripts(usize),
}

/// Generates the `Content-Security-Policy` header value for a page response.
pub fn generate_csp(
    mode: NetworkMode,
    allowlist: Option<&Allowlist>,
    page_script_hashes: &[CspHash],
    dev_server_origin: Option<&ParsedOrigin>,
) -> Result<String, CspError> {
    match mode {
        NetworkMode::UltraSafe => Ok(ULTRA_SAFE_CSP.to_string()),
        NetworkMode::Open => Ok(generate_open_csp(allowlist, dev_server_origin)),
        NetworkMode::Safe => generate_safe_csp(allowlist, page_script_hashes, dev_server_origin),
    }
}

fn generate_open_csp(
    allowlist: Option<&Allowlist>,
    dev_server_origin: Option<&ParsedOrigin>,
) -> String {
    let mut local_http_https = Vec::new();
    let mut local_ws = Vec::new();
    if let Some(al) = allowlist {
        for exc in &al.local_exceptions {
            let s = exc.origin.as_canonical();
            match exc.origin.scheme.as_str() {
                "http" | "https" => local_http_https.push(s),
                "ws" | "wss" => local_ws.push(s),
                _ => {}
            }
        }
    }
    if let Some(dev) = dev_server_origin {
        local_http_https.push(dev.as_canonical());
        if let Some(ws) = dev.to_websocket_origin() {
            local_ws.push(ws.as_canonical());
        }
    }

    let extra_http = join_suffix(&local_http_https);
    let extra_connect = {
        let mut all = local_http_https.clone();
        all.extend(local_ws);
        join_suffix(&all)
    };

    format!(
        "default-src 'self' https:{extra_http}; \
         script-src 'self' https:{extra_http} 'unsafe-inline' 'unsafe-eval'; \
         style-src 'self' https:{extra_http} 'unsafe-inline'; \
         img-src 'self' https:{extra_http} data: blob:; \
         font-src 'self' https:{extra_http} data:; \
         media-src 'self' https:{extra_http}; \
         connect-src 'self' https: wss:{extra_connect}; \
         frame-src 'self' https:{extra_http}; \
         object-src 'none'; \
         base-uri 'self'; \
         frame-ancestors 'none'; \
         form-action 'self' https:{extra_http};"
    )
}

fn generate_safe_csp(
    allowlist: Option<&Allowlist>,
    page_script_hashes: &[CspHash],
    dev_server_origin: Option<&ParsedOrigin>,
) -> Result<String, CspError> {
    if page_script_hashes.len() > MAX_INLINE_SCRIPTS_HASHED_PER_PAGE {
        return Err(CspError::TooManyInlineScripts(page_script_hashes.len()));
    }

    let empty = Allowlist::default();
    let al = allowlist.unwrap_or(&empty);

    let mut local_http_https = Vec::new();
    let mut local_ws = Vec::new();
    for exc in &al.local_exceptions {
        let s = exc.origin.as_canonical();
        match exc.origin.scheme.as_str() {
            "http" | "https" => local_http_https.push(s),
            "ws" | "wss" => local_ws.push(s),
            _ => {}
        }
    }
    if let Some(dev) = dev_server_origin {
        local_http_https.push(dev.as_canonical());
        if let Some(ws) = dev.to_websocket_origin() {
            local_ws.push(ws.as_canonical());
        }
    }

    let mut script_sources = vec!["'self'".to_string()];
    extend_patterns(&mut script_sources, &al.scripts);
    script_sources.extend(local_http_https.iter().cloned());
    for h in page_script_hashes {
        script_sources.push(h.as_csp_token());
    }

    let mut style_sources = vec!["'self'".to_string()];
    extend_patterns(&mut style_sources, &al.styles);
    style_sources.extend(local_http_https.iter().cloned());
    if al.inline_styles {
        style_sources.push("'unsafe-inline'".to_string());
    }

    let mut img_sources = vec!["'self'".to_string()];
    extend_patterns(&mut img_sources, &al.images);
    img_sources.extend(local_http_https.iter().cloned());
    if al.data_images {
        img_sources.push("data:".to_string());
    }

    let mut font_sources = vec!["'self'".to_string()];
    extend_patterns(&mut font_sources, &al.fonts);
    font_sources.extend(local_http_https.iter().cloned());

    let mut media_sources = vec!["'self'".to_string()];
    extend_patterns(&mut media_sources, &al.media);
    media_sources.extend(local_http_https.iter().cloned());

    let mut connect_sources = vec!["'self'".to_string()];
    extend_patterns(&mut connect_sources, &al.connect);
    connect_sources.extend(local_http_https.iter().cloned());
    connect_sources.extend(local_ws);

    let frame_sources = if al.frames.is_empty() && local_http_https.is_empty() {
        vec!["'none'".to_string()]
    } else {
        let mut v = Vec::new();
        extend_patterns(&mut v, &al.frames);
        v.extend(local_http_https);
        v
    };

    Ok(format!(
        "default-src 'self'; \
         script-src {scripts}; \
         style-src {styles}; \
         img-src {imgs}; \
         font-src {fonts}; \
         media-src {media}; \
         connect-src {connect}; \
         frame-src {frames}; \
         object-src 'none'; \
         base-uri 'self'; \
         frame-ancestors 'none'; \
         form-action 'self';",
        scripts = script_sources.join(" "),
        styles = style_sources.join(" "),
        imgs = img_sources.join(" "),
        fonts = font_sources.join(" "),
        media = media_sources.join(" "),
        connect = connect_sources.join(" "),
        frames = frame_sources.join(" "),
    ))
}

fn extend_patterns(out: &mut Vec<String>, patterns: &[AllowlistPattern]) {
    for p in patterns {
        out.push(p.as_canonical());
    }
}

fn join_suffix(items: &[String]) -> String {
    if items.is_empty() {
        String::new()
    } else {
        format!(" {}", items.join(" "))
    }
}
