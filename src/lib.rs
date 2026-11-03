//! Office: documents of the user's files in Collabora Online, for RustCloud.
//!
//! The plugin is a WOPI host ([`wopi`]) over the server's file API:
//! Collabora, a container of its own, reads and saves the one file the user
//! opened with a token the plugin gave it ([`session`]). Versions, quota, the
//! activity log and the trash stay the server's. The page (`ui/`) asks for
//! the editor's address and has the panel show it over itself.
//!
//! The logic runs on [`backend::Backend`], so `cargo test` checks it on the
//! host without WebAssembly.

wit_bindgen::generate!({ path: "wit", world: "app" });

pub mod backend;
pub mod discovery;
mod host;
pub mod session;
pub mod time;
pub mod web;
pub mod wopi;

use backend::{Backend, Reply};

pub const APP_ID: &str = "rc-office";

/// Answers a request to `/app/rc-office/…`.
pub fn route(backend: &dyn Backend, user: Option<&backend::User>, request: &backend::Request) -> Reply {
    let path = request.path.as_str();
    if let Some(rest) = path.strip_prefix("/wopi/files/") {
        return wopi::handle(backend, user, request, rest);
    }
    let html = "text/html; charset=utf-8";
    let js = "text/javascript; charset=utf-8";
    match (request.method.as_str(), path) {
        ("GET", "/") => web::asset(html, include_bytes!("../ui/index.html")),
        ("GET", "/edit") => web::asset(html, include_bytes!("../ui/edit.html")),
        ("GET", "/app.css") => web::asset("text/css; charset=utf-8", include_bytes!("../ui/app.css")),
        ("GET", "/i18n.js") => web::asset(js, include_bytes!("../ui/i18n.js")),
        ("GET", "/index.js") => web::asset(js, include_bytes!("../ui/index.js")),
        ("GET", "/edit.js") => web::asset(js, include_bytes!("../ui/edit.js")),
        ("GET", "/session" | "/status") if user.is_none() || backend.token().is_some() => {
            web::error(401, "unauthorized", "Not logged in")
        }
        ("GET", "/session") => session::open(backend, request),
        ("GET", "/status") => session::status(backend),
        _ => web::error(404, "not-found", "Not found"),
    }
}

struct Office;

impl Guest for Office {
    fn handle(user: Option<User>, request: Request) -> Response {
        let request = backend::Request {
            method: request.method,
            path: request.path,
            query: request.query,
            headers: request.headers.into_iter().map(|(name, value)| (name.to_ascii_lowercase(), value)).collect(),
            body: request.body,
        };
        let user = user.map(backend::User::from);
        let reply = route(&host::Host, user.as_ref(), &request);
        Response { status: reply.status, headers: reply.headers, body: reply.body }
    }
}

export!(Office);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::fake::Fake;

    fn get(fake: &Fake, logged_in: bool, path: &str) -> Reply {
        let user = backend::User { id: "u1".into(), username: "alice".into(), display_name: String::new() };
        let request = backend::Request { method: "GET".into(), path: path.into(), ..Default::default() };
        route(fake, logged_in.then_some(&user), &request)
    }

    #[test]
    fn serves_the_page() {
        let fake = Fake { token: None, ..Fake::default() };
        for path in ["/", "/edit", "/app.css", "/i18n.js", "/index.js", "/edit.js"] {
            assert_eq!(get(&fake, true, path).status, 200, "{path}");
        }
        assert_eq!(get(&fake, true, "/nothing").status, 404);
    }

    #[test]
    fn page_routes_need_the_user_not_a_token() {
        let fake = Fake { token: None, ..Fake::default() };
        assert_eq!(get(&fake, false, "/status").status, 401);
        assert_eq!(get(&fake, true, "/status").status, 200);
        // Collabora's token opens the document's WOPI routes, nothing else.
        let collabora = Fake::default();
        assert_eq!(get(&collabora, true, "/status").status, 401);
        assert_eq!(get(&collabora, true, "/session").status, 401);
    }
}
