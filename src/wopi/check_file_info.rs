//! CheckFileInfo: `GET /wopi/files/<id>` — what the file is and what the
//! user may do with it.

use serde_json::json;

use super::ITEM_VERSION;
use crate::backend::{Backend, FileError, Reply, Scope, User};
use crate::{session, web};

pub fn handle(backend: &dyn Backend, user: &User, scope: &Scope, id: &str) -> Reply {
    let file = match backend.file(id) {
        Ok(file) => file,
        Err(FileError::NotFound) => return web::empty(404),
        Err(err) => {
            backend.warn(&format!("CheckFileInfo: {err:?}"));
            return web::empty(500);
        }
    };
    let name = if user.display_name.is_empty() { &user.username } else { &user.display_name };
    let mut info = json!({
        "BaseFileName": file.name,
        "Size": file.size,
        "Version": file.etag,
        "LastModifiedTime": file.updated_at,
        "OwnerId": user.id,
        "UserId": user.id,
        "UserFriendlyName": name,
        "UserCanWrite": scope.write,
        "ReadOnly": !scope.write,
        "SupportsLocks": true,
        "SupportsGetLock": true,
        "SupportsUpdate": true,
        "SupportsExtendedLockLength": true,
        // No "save as" and renaming yet: the token reaches this one file.
        "UserCanNotWriteRelative": true,
        "SupportsRename": false,
        "UserCanRename": false,
        "EnableShare": false,
        "IsAnonymousUser": false,
    });
    // The panel, which shows the editor and talks to it.
    if let Some(origin) = session::origin_of(backend, id) {
        info["PostMessageOrigin"] = json!(origin);
    }
    web::json(200, info).with_header(ITEM_VERSION, file.etag)
}

#[cfg(test)]
mod tests {
    use crate::backend::fake::{Fake, FILE};
    use crate::backend::Scope;
    use crate::wopi::tests::call;

    fn info(fake: &Fake) -> serde_json::Value {
        let reply = call(fake, "GET", FILE, &[]);
        assert_eq!(reply.status, 200);
        assert_eq!(reply.header("X-WOPI-ItemVersion"), Some("etag-1"));
        serde_json::from_slice(&reply.body).unwrap()
    }

    #[test]
    fn describes_the_file() {
        let fake = Fake::default();
        fake.appdata.borrow_mut().insert(
            format!("sessions/{FILE}.json"),
            br#"{"origin":"https://cloud.example.com","updated_ms":1}"#.to_vec(),
        );
        let info = info(&fake);
        assert_eq!(info["BaseFileName"], "Report.docx");
        assert_eq!(info["Size"], 1200);
        assert_eq!(info["Version"], "etag-1");
        assert_eq!(info["LastModifiedTime"], "2026-11-03T10:00:00.123456Z");
        assert_eq!(info["UserId"], "u1");
        assert_eq!(info["UserFriendlyName"], "Alice Liddell");
        assert_eq!(info["UserCanWrite"], true);
        assert_eq!(info["ReadOnly"], false);
        assert_eq!(info["SupportsLocks"], true);
        assert_eq!(info["UserCanNotWriteRelative"], true);
        assert_eq!(info["PostMessageOrigin"], "https://cloud.example.com");
    }

    #[test]
    fn read_only_token() {
        let fake = Fake { token: Some(Scope { file: FILE.into(), write: false }), ..Fake::default() };
        let info = info(&fake);
        assert_eq!(info["UserCanWrite"], false);
        assert_eq!(info["ReadOnly"], true);
        assert!(info.get("PostMessageOrigin").is_none());
    }

    #[test]
    fn missing_file() {
        let fake = Fake::default();
        fake.files.borrow_mut().clear();
        assert_eq!(call(&fake, "GET", FILE, &[]).status, 404);
    }
}
