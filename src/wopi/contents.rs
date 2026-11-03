//! GetFile and PutFile: `/wopi/files/<id>/contents`. Neither passes the
//! document through the plugin's memory: the server sends the file itself
//! (`x-rustcloud-send`) and saves the request body into it
//! (`write-request-body`), of any size.

use serde_json::json;

use super::{conflict, ITEM_VERSION, LOCK};
use crate::backend::{Backend, FileError, Reply, Request, Scope};
use crate::{time, web};

/// What Collabora sends to say which version of the document it edited.
const TIMESTAMP: &str = "x-cool-wopi-timestamp";
/// Collabora's code for "the document changed in storage": it asks the user
/// whether to overwrite it.
const DOCUMENT_CHANGED: u32 = 1010;

pub fn get_file(backend: &dyn Backend, id: &str) -> Reply {
    match backend.file(id) {
        Ok(file) => web::empty(200)
            .with_header("x-rustcloud-send", format!("file:{id}"))
            .with_header("cache-control", "no-store")
            .with_header(ITEM_VERSION, file.etag),
        Err(FileError::NotFound) => web::empty(404),
        Err(err) => {
            backend.warn(&format!("GetFile: {err:?}"));
            web::empty(500)
        }
    }
}

pub fn put_file(backend: &dyn Backend, scope: &Scope, request: &Request, id: &str) -> Reply {
    if !scope.write {
        return web::empty(401);
    }
    let file = match backend.file(id) {
        Ok(file) => file,
        Err(FileError::NotFound) => return web::empty(404),
        Err(err) => {
            backend.warn(&format!("PutFile: {err:?}"));
            return web::empty(500);
        }
    };
    let held = match backend.get_lock(id) {
        Ok(held) => held,
        Err(err) => {
            backend.warn(&format!("PutFile: lock: {err:?}"));
            return web::empty(500);
        }
    };
    let lock = request.header(LOCK).map(str::trim).unwrap_or("");
    match &held {
        // Only the editor that holds the lock saves.
        Some(held) if held.app_id != crate::APP_ID || held.lock_id != lock => return conflict(Some(held)),
        Some(_) => {}
        // WOPI: an unlocked file is saved only while it is empty.
        None if file.size != 0 => return conflict(None),
        None => {}
    }
    // Collabora edited the version of this time; someone saved another since.
    if let Some(edited) = request.header(TIMESTAMP).map(str::trim).filter(|t| !t.is_empty()) {
        if !time::same(edited, &file.updated_at) {
            return changed();
        }
    }
    // The ETag of the same moment: a write that overtook ours fails here.
    match backend.write_body(id, Some(&file.etag)) {
        Ok(saved) => {
            web::json(200, json!({ "LastModifiedTime": saved.updated_at })).with_header(ITEM_VERSION, saved.etag)
        }
        Err(FileError::Conflict) => match backend.get_lock(id) {
            Ok(Some(held)) if held.app_id != crate::APP_ID => conflict(Some(&held)),
            _ => changed(),
        },
        Err(FileError::NotFound) => web::empty(404),
        Err(FileError::ReadOnly) => web::empty(401),
        Err(FileError::NoSpace | FileError::TooLarge) => web::empty(413),
        Err(err) => {
            backend.warn(&format!("PutFile: {err:?}"));
            web::empty(500)
        }
    }
}

fn changed() -> Reply {
    web::json(409, json!({ "COOLStatusCode": DOCUMENT_CHANGED }))
}

#[cfg(test)]
mod tests {
    use crate::backend::fake::{Fake, FILE};
    use crate::backend::Scope;
    use crate::wopi::tests::call;

    fn contents() -> String {
        format!("{FILE}/contents")
    }

    fn put(fake: &Fake, headers: &[(&str, &str)]) -> crate::backend::Reply {
        let mut all = vec![("X-WOPI-Override", "PUT")];
        all.extend_from_slice(headers);
        call(fake, "POST", &contents(), &all)
    }

    #[test]
    fn sends_the_file() {
        let fake = Fake::default();
        let reply = call(&fake, "GET", &contents(), &[]);
        assert_eq!(reply.status, 200);
        assert_eq!(reply.header("x-rustcloud-send"), Some(format!("file:{FILE}").as_str()));
        assert_eq!(reply.header("X-WOPI-ItemVersion"), Some("etag-1"));
        assert!(reply.body.is_empty());
    }

    #[test]
    fn saves_under_its_lock() {
        let fake = Fake::default();
        fake.lock_as(crate::APP_ID, "L1");
        let reply = put(&fake, &[("X-WOPI-Lock", "L1"), ("X-COOL-WOPI-Timestamp", "2026-11-03T10:00:00.123456Z")]);
        assert_eq!(reply.status, 200);
        let body: serde_json::Value = serde_json::from_slice(&reply.body).unwrap();
        assert_eq!(body["LastModifiedTime"], "2026-11-03T10:05:00.5Z");
        assert_eq!(reply.header("X-WOPI-ItemVersion"), Some("etag-2"));
        assert_eq!(fake.contents.borrow()[FILE], b"new content");
    }

    #[test]
    fn refuses_another_lock() {
        let fake = Fake::default();
        fake.lock_as(crate::APP_ID, "L1");
        let reply = put(&fake, &[("X-WOPI-Lock", "L2")]);
        assert_eq!(reply.status, 409);
        assert_eq!(reply.header("X-WOPI-Lock"), Some("L1"));
        let reply = put(&fake, &[]);
        assert_eq!(reply.status, 409);
        assert_eq!(*fake.writes.borrow(), 0);
    }

    #[test]
    fn refuses_another_apps_lock() {
        let fake = Fake::default();
        fake.lock_as("rc-notes", "N1");
        let reply = put(&fake, &[("X-WOPI-Lock", "N1")]);
        assert_eq!(reply.status, 409);
        assert_eq!(reply.header("X-WOPI-Lock"), Some(""));
        assert!(reply.header("X-WOPI-LockFailureReason").unwrap().contains("rc-notes"));
    }

    #[test]
    fn unlocked_only_when_empty() {
        let fake = Fake::default();
        let reply = put(&fake, &[("X-WOPI-Lock", "L1")]);
        assert_eq!(reply.status, 409);
        assert_eq!(reply.header("X-WOPI-Lock"), Some(""));
        fake.set_size(0);
        assert_eq!(put(&fake, &[]).status, 200);
    }

    #[test]
    fn changed_meanwhile() {
        let fake = Fake::default();
        fake.lock_as(crate::APP_ID, "L1");
        // Saved by someone else after Collabora loaded it.
        let reply = put(&fake, &[("X-WOPI-Lock", "L1"), ("X-COOL-WOPI-Timestamp", "2026-11-03T09:00:00Z")]);
        assert_eq!(reply.status, 409);
        let body: serde_json::Value = serde_json::from_slice(&reply.body).unwrap();
        assert_eq!(body["COOLStatusCode"], 1010);
        assert_eq!(*fake.writes.borrow(), 0);

        // The same instant written another way is the same version.
        let same = put(&fake, &[("X-WOPI-Lock", "L1"), ("X-COOL-WOPI-Timestamp", "2026-11-03T10:00:00.123456+00:00")]);
        assert_eq!(same.status, 200);
    }

    #[test]
    fn overtaken_between_check_and_write() {
        let fake = Fake::default();
        fake.lock_as(crate::APP_ID, "L1");
        *fake.race.borrow_mut() = true;
        let reply = put(&fake, &[("X-WOPI-Lock", "L1")]);
        assert_eq!(reply.status, 409);
        let body: serde_json::Value = serde_json::from_slice(&reply.body).unwrap();
        assert_eq!(body["COOLStatusCode"], 1010);
    }

    #[test]
    fn overwrite_without_timestamp() {
        // The user chose "overwrite": Collabora sends no timestamp.
        let fake = Fake::default();
        fake.lock_as(crate::APP_ID, "L1");
        assert_eq!(put(&fake, &[("X-WOPI-Lock", "L1")]).status, 200);
    }

    #[test]
    fn read_only() {
        let fake = Fake { token: Some(Scope { file: FILE.into(), write: false }), ..Fake::default() };
        assert_eq!(put(&fake, &[]).status, 401);
        assert_eq!(*fake.writes.borrow(), 0);
    }

    #[test]
    fn storage_full() {
        let fake = Fake { write_error: Some(crate::backend::FileError::NoSpace), ..Fake::default() };
        fake.lock_as(crate::APP_ID, "L1");
        assert_eq!(put(&fake, &[("X-WOPI-Lock", "L1")]).status, 413);
    }
}
