//! The WOPI host: what Collabora Online calls, with the token the plugin
//! gave it (`?access_token=`), on `/wopi/files/<id>` and
//! `/wopi/files/<id>/contents`. The server calls the plugin as the user the
//! token was given for, reaching only that one file.

mod check_file_info;
mod contents;
mod locks;

use crate::backend::{Backend, Lock, Reply, Request, User};
use crate::web;

/// How long a WOPI lock lasts without being refreshed (WOPI says 30 minutes).
pub const LOCK_SECONDS: u32 = 30 * 60;

pub const LOCK: &str = "x-wopi-lock";
pub const OLD_LOCK: &str = "x-wopi-oldlock";
pub const OVERRIDE: &str = "x-wopi-override";
pub const ITEM_VERSION: &str = "X-WOPI-ItemVersion";
pub const LOCK_FAILURE_REASON: &str = "X-WOPI-LockFailureReason";

/// Answers a request to `/wopi/files/<rest>`.
pub fn handle(backend: &dyn Backend, user: Option<&User>, request: &Request, rest: &str) -> Reply {
    let (id, contents) = match rest.split_once('/') {
        None => (rest, false),
        Some((id, "contents")) => (id, true),
        Some(_) => return web::empty(404),
    };
    if !web::valid_id(id) {
        return web::empty(404);
    }
    // Only Collabora with the token of this very file.
    let Some(scope) = backend.token() else {
        return web::empty(401);
    };
    if scope.file != id {
        return web::empty(401);
    }
    let Some(user) = user else {
        return web::empty(401);
    };
    let operation = request.header(OVERRIDE).unwrap_or("").trim().to_ascii_uppercase();
    match (request.method.as_str(), contents, operation.as_str()) {
        ("GET", false, _) => check_file_info::handle(backend, user, &scope, id),
        ("GET", true, _) => contents::get_file(backend, id),
        ("POST", true, "PUT" | "") => contents::put_file(backend, &scope, request, id),
        ("POST", false, "LOCK") => match request.header(OLD_LOCK) {
            Some(old) => locks::unlock_and_relock(backend, request, id, old),
            None => locks::lock(backend, request, id),
        },
        ("POST", false, "REFRESH_LOCK") => locks::refresh_lock(backend, request, id),
        ("POST", false, "UNLOCK") => locks::unlock(backend, request, id),
        ("POST", false, "GET_LOCK") => locks::get_lock(backend, id),
        // "Save as", renaming, deleting, user info: not yet.
        ("POST", _, _) => web::empty(501),
        _ => web::empty(405),
    }
}

/// `409 Conflict` for a lock someone else holds: its id if it is a WOPI lock
/// of this plugin, empty if another app holds the file, with a reason.
pub fn conflict(held: Option<&Lock>) -> Reply {
    let (lock, reason) = match held {
        None => (String::new(), "The file is not locked".to_owned()),
        Some(held) if held.app_id == crate::APP_ID => {
            (held.lock_id.clone(), "The file is locked by another editor".into())
        }
        Some(held) => (String::new(), format!("The file is being edited in the app \"{}\"", held.app_id)),
    };
    web::empty(409).with_header("X-WOPI-Lock", lock).with_header(LOCK_FAILURE_REASON, reason)
}

/// The `X-WOPI-Lock` header of a request; `400` if missing or empty.
fn lock_header(request: &Request) -> Result<&str, Reply> {
    match request.header(LOCK).map(str::trim) {
        Some(lock) if !lock.is_empty() && lock.len() <= 1024 => Ok(lock),
        _ => Err(web::empty(400)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::fake::{Fake, FILE};
    use crate::backend::Scope;

    pub fn user() -> User {
        User { id: "u1".into(), username: "alice".into(), display_name: "Alice Liddell".into() }
    }

    pub fn request(method: &str, path: &str, headers: &[(&str, &str)]) -> Request {
        Request {
            method: method.into(),
            path: path.into(),
            query: "access_token=t".into(),
            headers: headers.iter().map(|(k, v)| (k.to_ascii_lowercase(), (*v).to_owned())).collect(),
            body: Vec::new(),
        }
    }

    pub fn call(fake: &Fake, method: &str, rest: &str, headers: &[(&str, &str)]) -> Reply {
        handle(fake, Some(&user()), &request(method, &format!("/wopi/files/{rest}"), headers), rest)
    }

    #[test]
    fn needs_the_token_of_the_file() {
        let no_token = Fake { token: None, ..Fake::default() };
        assert_eq!(call(&no_token, "GET", FILE, &[]).status, 401);
        let other = Fake { token: Some(Scope { file: "other".into(), write: true }), ..Fake::default() };
        assert_eq!(call(&other, "GET", FILE, &[]).status, 401);
        let fake = Fake::default();
        assert_eq!(call(&fake, "GET", "../etc", &[]).status, 404);
        assert_eq!(call(&fake, "GET", &format!("{FILE}/other"), &[]).status, 404);
        let rest = FILE;
        let anonymous = handle(&fake, None, &request("GET", "/wopi/files/x", &[]), rest);
        assert_eq!(anonymous.status, 401);
    }

    #[test]
    fn refuses_what_it_does_not_do() {
        let fake = Fake::default();
        for operation in ["PUT_RELATIVE", "RENAME_FILE", "DELETE", "PUT_USER_INFO", "NONSENSE"] {
            assert_eq!(call(&fake, "POST", FILE, &[("X-WOPI-Override", operation)]).status, 501, "{operation}");
        }
        assert_eq!(call(&fake, "DELETE", FILE, &[]).status, 405);
    }
}
