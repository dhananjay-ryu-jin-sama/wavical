//! HTML5 and CSS3 parsers for scanning, inline script extraction, and vendor rewriting.
//! Uses `html5ever` (WHATWG HTML5 tokenizer) and `cssparser` (Servo CSS3 tokenizer).

use std::cell::RefCell;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use cssparser::{Parser, ParserInput, Token};
use html5ever::tokenizer::{
    BufferQueue, Tag, TagKind, Token as HtmlToken, TokenSink, TokenSinkResult, Tokenizer,
    TokenizerOpts,
};
use tendril::Tendril;
use wavical_netpolicy::html::{
    ExternalRef, ExternalRefCategory, HtmlScanResult, InlineHandlerFinding, InlineScriptBlock,
    StyleAttrFinding, hash_inline_script,
};

use crate::CliError;

struct ScanSink {
    file: String,
    result: RefCell<HtmlScanResult>,
    in_script: RefCell<Option<PendingScript>>,
    script_buf: RefCell<String>,
}

struct PendingScript {
    line: usize,
    is_external: bool,
    is_module: bool,
}

impl TokenSink for ScanSink {
    type Handle = ();

    fn process_token(&self, token: HtmlToken, line_number: u64) -> TokenSinkResult<()> {
        let line = line_number as usize;
        match token {
            HtmlToken::TagToken(Tag {
                kind: TagKind::StartTag,
                name,
                attrs,
                self_closing: _,
                had_duplicate_attributes: _,
            }) => {
                let tag_name = name.as_ref().to_ascii_lowercase();
                let mut src_val: Option<String> = None;
                let mut href_val: Option<String> = None;
                let mut rel_val: Option<String> = None;
                let mut type_val: Option<String> = None;
                let mut has_integrity = false;

                for attr in &attrs {
                    let attr_name = attr.name.local.as_ref().to_ascii_lowercase();
                    let attr_val = attr.value.as_ref().to_string();

                    if attr_name.starts_with("on") && attr_name.len() > 2 {
                        self.result
                            .borrow_mut()
                            .inline_handlers
                            .push(InlineHandlerFinding {
                                file: self.file.clone(),
                                line,
                                column: 1,
                                tag: tag_name.clone(),
                                attribute: attr_name,
                            });
                        continue;
                    }
                    if attr_name == "style" {
                        self.result
                            .borrow_mut()
                            .style_attributes
                            .push(StyleAttrFinding {
                                file: self.file.clone(),
                                line,
                                tag: tag_name.clone(),
                            });
                    }
                    match attr_name.as_str() {
                        "src" => src_val = Some(attr_val),
                        "href" => href_val = Some(attr_val),
                        "rel" => rel_val = Some(attr_val.to_ascii_lowercase()),
                        "type" => type_val = Some(attr_val.to_ascii_lowercase()),
                        "integrity" => has_integrity = !attr_val.trim().is_empty(),
                        _ => {}
                    }
                }

                if tag_name == "script" {
                    let is_external = src_val.is_some();
                    let is_module = type_val.as_deref() == Some("module");
                    *self.in_script.borrow_mut() = Some(PendingScript {
                        line,
                        is_external,
                        is_module,
                    });
                    self.script_buf.borrow_mut().clear();
                    if let Some(src) = src_val
                        && is_external_http_url(&src)
                    {
                        self.result.borrow_mut().external_refs.push(ExternalRef {
                            category: ExternalRefCategory::Script,
                            url: src,
                            line,
                            has_integrity,
                        });
                    }
                } else if tag_name == "link" {
                    if let Some(href) = href_val
                        && is_external_http_url(&href)
                    {
                        let rel = rel_val.as_deref().unwrap_or("");
                        if rel.split_whitespace().any(|r| r == "stylesheet") {
                            self.result.borrow_mut().external_refs.push(ExternalRef {
                                category: ExternalRefCategory::Style,
                                url: href,
                                line,
                                has_integrity,
                            });
                        }
                    }
                } else if tag_name == "img" {
                    if let Some(src) = src_val
                        && is_external_http_url(&src)
                    {
                        self.result.borrow_mut().external_refs.push(ExternalRef {
                            category: ExternalRefCategory::Image,
                            url: src,
                            line,
                            has_integrity,
                        });
                    }
                } else if matches!(tag_name.as_str(), "video" | "audio" | "source") {
                    if let Some(src) = src_val
                        && is_external_http_url(&src)
                    {
                        self.result.borrow_mut().external_refs.push(ExternalRef {
                            category: ExternalRefCategory::Media,
                            url: src,
                            line,
                            has_integrity,
                        });
                    }
                } else if tag_name == "iframe"
                    && let Some(src) = src_val
                    && is_external_http_url(&src)
                {
                    self.result.borrow_mut().external_refs.push(ExternalRef {
                        category: ExternalRefCategory::Frame,
                        url: src,
                        line,
                        has_integrity,
                    });
                }
                let _ = self.in_script.borrow().as_ref().map(|p| p.is_module);
            }
            HtmlToken::TagToken(Tag {
                kind: TagKind::EndTag,
                name,
                ..
            }) => {
                if name.as_ref().eq_ignore_ascii_case("script")
                    && let Some(pending) = self.in_script.borrow_mut().take()
                {
                    let content = std::mem::take(&mut *self.script_buf.borrow_mut());
                    if !pending.is_external {
                        let idx = self.result.borrow().inline_scripts.len();
                        let hash = hash_inline_script(content.as_bytes());
                        let byte_len = content.len();
                        self.result
                            .borrow_mut()
                            .inline_scripts
                            .push(InlineScriptBlock {
                                index: idx,
                                line: pending.line,
                                byte_len,
                                hash,
                                content,
                            });
                    }
                }
            }
            HtmlToken::CharacterTokens(chars) if self.in_script.borrow().is_some() => {
                self.script_buf.borrow_mut().push_str(chars.as_ref());
            }
            _ => {}
        }
        TokenSinkResult::Continue
    }
}

fn is_external_http_url(raw: &str) -> bool {
    let Ok(parsed) = url::Url::parse(raw.trim()) else {
        return false;
    };
    matches!(parsed.scheme(), "http" | "https")
}

/// Scans an HTML document using `html5ever` and fills the WP2 `HtmlScanResult`.
#[must_use]
pub fn scan_html(file_rel: &str, html: &str) -> HtmlScanResult {
    let sink = ScanSink {
        file: file_rel.to_string(),
        result: RefCell::new(HtmlScanResult {
            file: file_rel.to_string(),
            ..Default::default()
        }),
        in_script: RefCell::new(None),
        script_buf: RefCell::new(String::new()),
    };

    let tok = Tokenizer::new(sink, TokenizerOpts::default());
    let queue = BufferQueue::default();
    queue.push_back(Tendril::from_slice(html));
    let _ = tok.feed(&queue);
    tok.end();

    tok.sink.result.into_inner()
}

/// CSS reference found by `cssparser` (`url(...)` or `@import`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CssRefSpan {
    pub url: String,
    pub start: usize,
    pub end: usize,
    pub is_import: bool,
}

/// Extracts `url(...)` and `@import "..."` references from CSS using `cssparser`.
#[must_use]
pub fn scan_css_refs(css: &str) -> Vec<CssRefSpan> {
    let mut out = Vec::new();
    let mut input = ParserInput::new(css);
    let mut parser = Parser::new(&mut input);
    collect_css_tokens(&mut parser, css, &mut out);
    out
}

fn collect_css_tokens<'i, 't>(
    parser: &mut Parser<'i, 't>,
    full_css: &str,
    out: &mut Vec<CssRefSpan>,
) {
    let mut after_import = false;
    while !parser.is_exhausted() {
        let start_pos = parser.position().byte_index();
        let Ok(tok) = parser.next_including_whitespace_and_comments() else {
            break;
        };
        enum TokKind {
            Skip,
            Import,
            Quoted(String),
            Unquoted(String),
            UrlFn,
            Block,
            Other,
        }
        let kind = match tok {
            Token::WhiteSpace(_) | Token::Comment(_) => TokKind::Skip,
            Token::AtKeyword(kw) if kw.eq_ignore_ascii_case("import") => TokKind::Import,
            Token::QuotedString(s) if after_import => TokKind::Quoted(s.as_ref().to_string()),
            Token::UnquotedUrl(u) => TokKind::Unquoted(u.as_ref().to_string()),
            Token::Function(name) if name.eq_ignore_ascii_case("url") => TokKind::UrlFn,
            Token::ParenthesisBlock | Token::SquareBracketBlock | Token::CurlyBracketBlock => {
                TokKind::Block
            }
            _ => TokKind::Other,
        };
        let end_pos = parser.position().byte_index();
        match kind {
            TokKind::Skip => {}
            TokKind::Import => {
                after_import = true;
            }
            TokKind::Quoted(s) => {
                let raw_slice = &full_css[start_pos..end_pos];
                if let Some((q_start, q_end)) = find_inner_quoted_span(raw_slice, &s) {
                    out.push(CssRefSpan {
                        url: s,
                        start: start_pos + q_start,
                        end: start_pos + q_end,
                        is_import: true,
                    });
                }
                after_import = false;
            }
            TokKind::Unquoted(u) => {
                let raw_slice = &full_css[start_pos..end_pos];
                if let Some((u_start, u_end)) = find_inner_url_span(raw_slice, &u) {
                    out.push(CssRefSpan {
                        url: u,
                        start: start_pos + u_start,
                        end: start_pos + u_end,
                        is_import: after_import,
                    });
                }
                after_import = false;
            }
            TokKind::UrlFn => {
                let was_import = after_import;
                after_import = false;
                let _ = parser.parse_nested_block(|nested| {
                    let s_pos = nested.position().byte_index();
                    let found = match nested.next() {
                        Ok(Token::QuotedString(u)) => Some(u.as_ref().to_string()),
                        _ => None,
                    };
                    if let Some(u) = found {
                        let e_pos = nested.position().byte_index();
                        let raw_slice = &full_css[s_pos..e_pos];
                        if let Some((q_start, q_end)) = find_inner_quoted_span(raw_slice, &u) {
                            out.push(CssRefSpan {
                                url: u,
                                start: s_pos + q_start,
                                end: s_pos + q_end,
                                is_import: was_import,
                            });
                        }
                    }
                    Ok::<(), cssparser::ParseError<'i, ()>>(())
                });
            }
            TokKind::Block => {
                after_import = false;
                let _ = parser.parse_nested_block(|nested| {
                    collect_css_tokens(nested, full_css, out);
                    Ok::<(), cssparser::ParseError<'i, ()>>(())
                });
            }
            TokKind::Other => {
                after_import = false;
            }
        }
    }
}

fn find_inner_quoted_span(raw_slice: &str, value: &str) -> Option<(usize, usize)> {
    let idx = raw_slice.find(value)?;
    Some((idx, idx + value.len()))
}

fn find_inner_url_span(raw_slice: &str, value: &str) -> Option<(usize, usize)> {
    let idx = raw_slice.find(value)?;
    Some((idx, idx + value.len()))
}

/// Result of extracting inline `<script>` blocks for `ultra-safe` mode.
#[derive(Debug, Clone)]
pub struct ExtractedHtmlOutput {
    pub rewritten_html: String,
    pub extracted_files: Vec<(String, String)>,
    pub scan: HtmlScanResult,
}

/// Extracts inline `<script>` blocks from `html` into `.wavical-extracted/` files,
/// preserving execution order and `type="module"` vs classic semantics.
pub fn extract_inline_scripts_from_html(
    file_rel: &str,
    html: &str,
    extracted_url_prefix: &str,
) -> ExtractedHtmlOutput {
    let scan = scan_html(file_rel, html);
    if scan.inline_scripts.is_empty() {
        return ExtractedHtmlOutput {
            rewritten_html: html.to_string(),
            extracted_files: Vec::new(),
            scan,
        };
    }

    let safe_stem = file_rel
        .replace(['/', '\\', '.'], "_")
        .trim_matches('_')
        .to_string();

    // Walk `<script` ... `</script>` tags matching the scanned inline scripts in order.
    let mut rewritten = String::with_capacity(html.len() + 256);
    let mut extracted_files = Vec::new();
    let mut cursor = 0usize;
    let mut inline_idx = 0usize;
    let lower = html.to_ascii_lowercase();

    while let Some(rel_open) = lower[cursor..].find("<script") {
        // Skip HTML comments <!-- ... --> that may precede <script
        if let Some(rel_cmt) = lower[cursor..].find("<!--")
            && rel_cmt < rel_open
        {
            let cmt_start = cursor + rel_cmt;
            if let Some(rel_end) = lower[cmt_start + 4..].find("-->") {
                let cmt_end = cmt_start + 4 + rel_end + 3;
                rewritten.push_str(&html[cursor..cmt_end]);
                cursor = cmt_end;
                continue;
            }
        }
        let open_start = cursor + rel_open;
        let after_kw = open_start + "<script".len();
        let next_ch = lower.as_bytes().get(after_kw).copied();
        if !matches!(next_ch, Some(b'>' | b' ' | b'\t' | b'\r' | b'\n' | b'/')) {
            rewritten.push_str(&html[cursor..after_kw]);
            cursor = after_kw;
            continue;
        }
        let Some(rel_gt) = html[after_kw..].find('>') else {
            break;
        };
        let open_end = after_kw + rel_gt + 1;
        let tag_attrs = &html[after_kw..open_end - 1];
        let tag_attrs_lower = tag_attrs.to_ascii_lowercase();

        let Some(rel_close) = lower[open_end..].find("</script>") else {
            break;
        };
        let close_start = open_end + rel_close;
        let close_end = close_start + "</script>".len();

        let has_src = has_attr(&tag_attrs_lower, "src");
        if has_src {
            rewritten.push_str(&html[cursor..close_end]);
            cursor = close_end;
            continue;
        }

        let body = &html[open_end..close_start];
        let js_name = format!("{safe_stem}_inline_{inline_idx}.js");
        inline_idx += 1;
        extracted_files.push((js_name.clone(), body.to_string()));

        let is_module = tag_attrs_lower.contains("type=\"module\"")
            || tag_attrs_lower.contains("type='module'")
            || tag_attrs_lower.contains("type=module");

        rewritten.push_str(&html[cursor..open_start]);
        if is_module {
            rewritten.push_str(&format!(
                "<script type=\"module\" src=\"{extracted_url_prefix}{js_name}\"></script>"
            ));
        } else {
            rewritten.push_str(&format!(
                "<script src=\"{extracted_url_prefix}{js_name}\"></script>"
            ));
        }
        cursor = close_end;
    }

    rewritten.push_str(&html[cursor..]);
    ExtractedHtmlOutput {
        rewritten_html: rewritten,
        extracted_files,
        scan,
    }
}

fn has_attr(attrs_lower: &str, name: &str) -> bool {
    let mut rest = attrs_lower;
    while let Some(idx) = rest.find(name) {
        let before_ok = idx == 0 || rest.as_bytes()[idx - 1].is_ascii_whitespace();
        let after = &rest[idx + name.len()..];
        let after_ok = after.trim_start().starts_with('=');
        if before_ok && after_ok {
            return true;
        }
        rest = &rest[idx + name.len()..];
    }
    false
}

/// Extracts all inline `<script>` blocks across HTML files in `frontend_dir` into `out_extracted_dir`
/// (e.g. `src/.wavical-extracted/` for `dev-ultra-safe` or `dist/.wavical-extracted/` for `build-ultra-safe`).
pub fn extract_frontend_inline_scripts(
    frontend_dir: &Path,
    extracted_dir: &Path,
    extracted_url_prefix: &str,
    rewrite_in_place: bool,
) -> Result<Vec<HtmlScanResult>, CliError> {
    if !frontend_dir.is_dir() {
        return Ok(Vec::new());
    }
    if extracted_dir.exists() {
        let _ = fs::remove_dir_all(extracted_dir);
    }

    let mut html_files = Vec::new();
    collect_html_files(frontend_dir, frontend_dir, &mut html_files)?;

    let mut scans = Vec::new();
    let mut created_dir = false;
    let mut seen_names = HashSet::new();

    for (rel, full_path) in html_files {
        let raw =
            fs::read_to_string(&full_path).map_err(|e| CliError::ShellError(e.to_string()))?;
        let out = extract_inline_scripts_from_html(&rel, &raw, extracted_url_prefix);
        if !out.extracted_files.is_empty() {
            if !created_dir {
                fs::create_dir_all(extracted_dir).map_err(|e| CliError::CreateDirFailed {
                    path: extracted_dir.to_path_buf(),
                    source: e,
                })?;
                created_dir = true;
            }
            for (name, js_body) in &out.extracted_files {
                if seen_names.insert(name.clone()) {
                    let dst = extracted_dir.join(name);
                    fs::write(&dst, js_body).map_err(|e| CliError::WriteFileFailed {
                        path: dst,
                        source: e,
                    })?;
                }
            }
            if rewrite_in_place {
                fs::write(&full_path, &out.rewritten_html).map_err(|e| {
                    CliError::WriteFileFailed {
                        path: full_path,
                        source: e,
                    }
                })?;
            }
        }
        scans.push(out.scan);
    }

    Ok(scans)
}

fn collect_html_files(
    root: &Path,
    cur: &Path,
    out: &mut Vec<(String, PathBuf)>,
) -> Result<(), CliError> {
    let entries = fs::read_dir(cur).map_err(|e| CliError::ShellError(e.to_string()))?;
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') || name == "vendor" || name == "node_modules" {
            continue;
        }
        if path.is_dir() {
            collect_html_files(root, &path, out)?;
        } else if path.is_file()
            && path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "html" | "htm"))
        {
            let rel = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            out.push((rel, path));
        }
    }
    Ok(())
}
