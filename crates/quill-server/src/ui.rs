use std::path::{Component, Path, PathBuf};

use axum::body::Body;
use axum::http::{header, StatusCode, Uri};
use axum::response::{IntoResponse, Response};

const INDEX: &str = "index.html";

fn mime_of(name: &str) -> &'static str {
    match name.rsplit('.').next().unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "json" => "application/json; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "ico" => "image/x-icon",
        "webp" => "image/webp",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "map" => "application/json; charset=utf-8",
        "wasm" => "application/wasm",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

/// 把 URL 路径安全地落到 root 之下。`.`、`..`、绝对前缀、盘符一律拒绝。
fn safe_join(root: &Path, url_path: &str) -> Option<PathBuf> {
    let decoded = percent_decode(url_path);
    let rel = decoded.trim_start_matches('/');
    if rel.is_empty() {
        return None;
    }

    let mut out = root.to_path_buf();
    for comp in Path::new(rel).components() {
        match comp {
            Component::Normal(part) => {
                let s = part.to_str()?;
                if s.is_empty() || s == "." || s == ".." {
                    return None;
                }
                if s.contains(':') {
                    return None;
                }
                out.push(s);
            }
            Component::CurDir => {}
            _ => return None,
        }
    }
    Some(out)
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            let hi = (b[i + 1] as char).to_digit(16);
            let lo = (b[i + 2] as char).to_digit(16);
            if let (Some(h), Some(l)) = (hi, lo) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub async fn serve(root: &Path, uri: Uri) -> Response {
    let path = uri.path();

    if let Some(target) = safe_join(root, path) {
        if target.is_file() && crate::pathsafe::is_within(root, &target) {
            return send_file(&target);
        }
    }

    let index = root.join(INDEX);
    if index.is_file() {
        return send_file(&index);
    }

    (
        StatusCode::NOT_FOUND,
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        format!(
            "前端产物不可用：{} 下没有 {INDEX}。\n\
             下一步：在 ui/web 目录执行 `npm ci && npm run build`，\
             或用 QUILL_WEB_DIR 指向已有产物目录。后端 API 不受影响。",
            root.display()
        ),
    )
        .into_response()
}

fn send_file(target: &Path) -> Response {
    match std::fs::read(target) {
        Ok(bytes) => {
            let ct = mime_of(&target.to_string_lossy());
            (
                StatusCode::OK,
                [
                    (header::CONTENT_TYPE, ct),
                    (header::CACHE_CONTROL, "no-cache"),
                ],
                Body::from(bytes),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
            format!("读取 {} 失败：{e}", target.display()),
        )
            .into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_names_resolve_under_root() {
        let root = Path::new("/srv/ui");
        assert_eq!(
            safe_join(root, "/app.js"),
            Some(PathBuf::from("/srv/ui/app.js"))
        );
        assert_eq!(
            safe_join(root, "/assets/logo.svg"),
            Some(PathBuf::from("/srv/ui/assets/logo.svg"))
        );
    }

    #[test]
    fn traversal_in_every_shape_is_refused() {
        let root = Path::new("/srv/ui");
        for bad in [
            "/../etc/passwd",
            "/a/../../etc/passwd",
            "/./../secret",
            "/a/b/../../../x",
            "//etc/passwd",
            "/C:/Windows/system32",
            "/a/..%2f..%2fb",
        ] {
            let joined = safe_join(root, bad);
            if let Some(p) = &joined {
                assert!(
                    p.starts_with(root),
                    "{bad:?} 逃出了根目录：{}",
                    p.display()
                );
            }
        }
        assert_eq!(safe_join(root, "/../etc/passwd"), None, "上级目录必须直接拒绝");
        assert_eq!(safe_join(root, "/a/../../etc/passwd"), None);
        assert_eq!(safe_join(root, "/C:/Windows"), None, "盘符必须拒绝");
    }

    #[test]
    fn percent_encoded_names_are_decoded() {
        let root = Path::new("/srv/ui");
        assert_eq!(
            safe_join(root, "/%61%70%70.js"),
            Some(PathBuf::from("/srv/ui/app.js")),
            "%61%70%70.js 应解码成 app.js"
        );
    }

    #[test]
    fn empty_path_is_not_a_file_request() {
        assert_eq!(safe_join(Path::new("/srv/ui"), "/"), None);
        assert_eq!(safe_join(Path::new("/srv/ui"), ""), None);
    }

    #[test]
    fn mime_covers_the_assets_a_vendored_spa_needs() {
        assert_eq!(mime_of("index.html"), "text/html; charset=utf-8");
        assert_eq!(mime_of("app.js"), "text/javascript; charset=utf-8");
        assert_eq!(mime_of("style.css"), "text/css; charset=utf-8");
        assert_eq!(mime_of("f.woff2"), "font/woff2");
        assert_eq!(mime_of("m.wasm"), "application/wasm");
        assert_eq!(mime_of("noext"), "application/octet-stream");
    }
}
