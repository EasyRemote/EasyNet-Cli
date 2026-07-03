// EasyNet CLI — Pages reference system: MIME allow-list
// =====================================================
//
// File: src/daemon/ability/builtins/resources/pages/mime.rs
// Description: extension → Content-Type table for `page.fetch`.
//              Allow-list, not detect-by-content. Anything not
//              on the list serves as `application/octet-stream`
//              with `Content-Disposition: attachment` so the
//              browser cannot interpret it as code.
//
// Conformance: RFC-006-B v0.6 §6.4.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

/// One MIME match. `force_attachment` flips the Hub's `Content-Disposition`
/// to `attachment` so the browser downloads rather than renders. The
/// `attachment` flag is `true` for everything outside the allow-list.
#[derive(Debug, Clone, Copy)]
pub struct Mime {
    pub content_type: &'static str,
    pub force_attachment: bool,
}

impl Mime {
    const fn ok(ct: &'static str) -> Self {
        Self {
            content_type: ct,
            force_attachment: false,
        }
    }
    const fn download() -> Self {
        Self {
            content_type: "application/octet-stream",
            force_attachment: true,
        }
    }
}

/// Look up the MIME for a path's extension. Case-insensitive.
/// Returns `Mime::download()` for anything not in the table; that
/// is the safe default: the browser will save the bytes rather
/// than execute them.
pub fn mime_from_path(path: &str) -> Mime {
    let ext = path.rsplit('.').next().unwrap_or("").to_ascii_lowercase();

    match ext.as_str() {
        "html" | "htm" => Mime::ok("text/html; charset=utf-8"),
        "css" => Mime::ok("text/css; charset=utf-8"),
        "js" | "mjs" => Mime::ok("application/javascript; charset=utf-8"),
        "json" => Mime::ok("application/json; charset=utf-8"),
        "svg" => Mime::ok("image/svg+xml"),
        "png" => Mime::ok("image/png"),
        "jpg" | "jpeg" => Mime::ok("image/jpeg"),
        "gif" => Mime::ok("image/gif"),
        "webp" => Mime::ok("image/webp"),
        "woff" => Mime::ok("font/woff"),
        "woff2" => Mime::ok("font/woff2"),
        "txt" => Mime::ok("text/plain; charset=utf-8"),
        "md" => Mime::ok("text/plain; charset=utf-8"),
        _ => Mime::download(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_renders() {
        let m = mime_from_path("/hello-world.html");
        assert_eq!(m.content_type, "text/html; charset=utf-8");
        assert!(!m.force_attachment);
    }

    #[test]
    fn css_renders() {
        let m = mime_from_path("/style.css");
        assert_eq!(m.content_type, "text/css; charset=utf-8");
    }

    #[test]
    fn unknown_extension_downloads() {
        let m = mime_from_path("/secret.exe");
        assert_eq!(m.content_type, "application/octet-stream");
        assert!(m.force_attachment);
    }

    #[test]
    fn case_insensitive() {
        assert_eq!(mime_from_path("/PHOTO.JPG").content_type, "image/jpeg",);
    }

    #[test]
    fn no_extension_downloads() {
        let m = mime_from_path("/README");
        assert!(m.force_attachment);
    }
}
