//! The console page as `npm run build` leaves it in dist/, which a packaged
//! XenoNet serves (server/static.js), with its Content-Security-Policy.

use crate::http::{decode_uri_component, Reply};
use std::path::{Component, Path, PathBuf};

/// The page's Content-Security-Policy: scripts only from XenoNet's own
/// server, nothing inline or evaluated, WebAssembly may compile, styles
/// may be inline, images data: and blob:, fonts data:, no framing, no
/// forms, connections only to XenoNet's own server.
pub const CSP: &str = "default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; worker-src 'self' blob:; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; font-src 'self' data:; connect-src 'self'; object-src 'none'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'";

fn mime(file: &Path) -> &'static str {
    match file.extension().and_then(|e| e.to_str()).map(|e| format!(".{e}")).as_deref() {
        Some(".html") => "text/html",
        Some(".js") => "text/javascript",
        Some(".css") => "text/css",
        Some(".svg") => "image/svg+xml",
        Some(".woff2") => "font/woff2",
        Some(".woff") => "font/woff",
        Some(".png") => "image/png",
        Some(".ico") => "image/x-icon",
        Some(".json") => "application/json",
        _ => "application/octet-stream",
    }
}

/// path.normalize(path.join(dist, rel)), lexically.
fn joined(dist: &Path, rel: &str) -> PathBuf {
    let mut out = dist.to_path_buf();
    for c in Path::new(rel.trim_start_matches(['/', '\\'])).components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::Normal(p) => out.push(p),
            _ => {}
        }
    }
    out
}

/// The file a request's path names in `dist`: a path that isn't a file
/// there gets the page, which routes for itself.
pub fn serve(dist: &Path, pathname: &str) -> Reply {
    let Some(rel) = decode_uri_component(pathname) else { return Reply::error(400, "bad path") };
    let mut file = joined(dist, &rel);
    // Inside dist, not in a folder beside it whose name starts the same.
    if file != dist && !file.starts_with(dist) {
        return Reply::error(403, "forbidden");
    }
    if !file.is_file() {
        file = dist.join("index.html");
    }
    let Ok(data) = std::fs::read(&file) else {
        return Reply::raw(503, "Run `npm run build` first, or use `npm run dev`.", &[("content-type", "text/plain")]);
    };
    let kind = mime(&file);
    let mut r = Reply::raw(200, data, &[]);
    r.headers = vec![("content-type".into(), kind.into())];
    if kind == "text/html" {
        r.headers.push(("content-security-policy".into(), CSP.into()));
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    // The cases of test/static.test.js.
    #[test]
    fn files_from_dist_and_the_page_for_any_other_path() {
        let root = std::env::temp_dir().join(format!("xenonet-static-{}", std::process::id()));
        let dist = root.join("dist");
        std::fs::create_dir_all(dist.join("assets")).unwrap();
        std::fs::create_dir_all(root.join("dist-private")).unwrap();
        std::fs::write(dist.join("index.html"), "<!doctype html>").unwrap();
        std::fs::write(dist.join("assets/app.js"), "export {}").unwrap();
        std::fs::write(root.join("package.json"), "{}").unwrap();
        std::fs::write(root.join("dist-private/secret.txt"), "not for the page").unwrap();
        let r = serve(&dist, "/assets/app.js");
        assert_eq!((r.status, r.header("content-type"), r.header("content-security-policy")), (200, Some("text/javascript"), None));
        for page in ["/", "/assets", "/some/route"] {
            let r = serve(&dist, page);
            assert_eq!((r.status, r.header("content-type"), r.header("content-security-policy")), (200, Some("text/html"), Some(CSP)), "{page}");
        }
        assert_eq!(serve(&dist, "/assets/app%2Ejs").header("content-type"), Some("text/javascript"));
        assert_eq!(serve(&dist, "/..%2fpackage.json").status, 403);
        assert_eq!(serve(&dist, "/..%2F..%2F..%2Fetc%2Fpasswd").status, 403);
        assert_eq!(serve(&dist, "/..%2fdist-private%2fsecret.txt").status, 403);
        let r = serve(&dist, "/%E0%A4%A");
        assert_eq!((r.status, r.body_json()), (400, serde_json::json!({ "error": "bad path" })));
        std::fs::remove_file(dist.join("index.html")).unwrap();
        assert_eq!(serve(&dist, "/").status, 503);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
