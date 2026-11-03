//! WOPI locks: `POST /wopi/files/<id>` with `X-WOPI-Override` `LOCK`,
//! `REFRESH_LOCK`, `UNLOCK`, `GET_LOCK` and `LOCK` with `X-WOPI-OldLock`
//! (UnlockAndRelock), over the server's file locks. While Collabora holds
//! one, the panel and other apps can't change the file.

use super::{conflict, lock_header, ITEM_VERSION, LOCK_SECONDS};
use crate::backend::{Backend, FileError, Lock, LockError, Reply, Request};
use crate::web;

pub fn lock(backend: &dyn Backend, request: &Request, id: &str) -> Reply {
    let lock = match lock_header(request) {
        Ok(lock) => lock,
        Err(reply) => return reply,
    };
    // Taken, or renewed when it is this lock already.
    take(backend, id, lock)
}

pub fn refresh_lock(backend: &dyn Backend, request: &Request, id: &str) -> Reply {
    let lock = match lock_header(request) {
        Ok(lock) => lock,
        Err(reply) => return reply,
    };
    match held(backend, id) {
        Ok(Some(held)) if is_ours(&held, lock) => take(backend, id, lock),
        // Unlocked, or someone else's: WOPI says 409.
        Ok(held) => conflict(held.as_ref()),
        Err(reply) => reply,
    }
}

pub fn unlock(backend: &dyn Backend, request: &Request, id: &str) -> Reply {
    let lock = match lock_header(request) {
        Ok(lock) => lock,
        Err(reply) => return reply,
    };
    match held(backend, id) {
        Ok(Some(held)) if is_ours(&held, lock) => match backend.unlock(id, lock) {
            Ok(()) => with_version(backend, id, web::empty(200)),
            Err(err) => lock_failure(backend, err),
        },
        Ok(held) => conflict(held.as_ref()),
        Err(reply) => reply,
    }
}

pub fn get_lock(backend: &dyn Backend, id: &str) -> Reply {
    match held(backend, id) {
        // Another app's lock is no WOPI lock: there is none to give.
        Ok(Some(held)) if held.app_id == crate::APP_ID => web::empty(200).with_header("X-WOPI-Lock", held.lock_id),
        Ok(_) => web::empty(200).with_header("X-WOPI-Lock", ""),
        Err(reply) => reply,
    }
}

pub fn unlock_and_relock(backend: &dyn Backend, request: &Request, id: &str, old: &str) -> Reply {
    let lock = match lock_header(request) {
        Ok(lock) => lock,
        Err(reply) => return reply,
    };
    let old = old.trim();
    match held(backend, id) {
        Ok(Some(held)) if is_ours(&held, old) => {
            if let Err(err) = backend.unlock(id, old) {
                return lock_failure(backend, err);
            }
            take(backend, id, lock)
        }
        Ok(held) => conflict(held.as_ref()),
        Err(reply) => reply,
    }
}

fn is_ours(held: &Lock, lock: &str) -> bool {
    held.app_id == crate::APP_ID && held.lock_id == lock
}

fn take(backend: &dyn Backend, id: &str, lock: &str) -> Reply {
    match backend.lock(id, lock, LOCK_SECONDS) {
        Ok(()) => with_version(backend, id, web::empty(200)),
        Err(err) => lock_failure(backend, err),
    }
}

/// The lock held on the file, or the answer if there is no file.
fn held(backend: &dyn Backend, id: &str) -> Result<Option<Lock>, Reply> {
    backend.get_lock(id).map_err(|err| match err {
        FileError::NotFound => web::empty(404),
        err => {
            backend.warn(&format!("lock of {id}: {err:?}"));
            web::empty(500)
        }
    })
}

fn lock_failure(backend: &dyn Backend, err: LockError) -> Reply {
    match err {
        LockError::Locked(held) => conflict(Some(&held)),
        LockError::NotFound => web::empty(404),
        LockError::ReadOnly => web::empty(409)
            .with_header("X-WOPI-Lock", "")
            .with_header(super::LOCK_FAILURE_REASON, "The file is read-only"),
        LockError::Other(why) => {
            backend.warn(&format!("lock: {why}"));
            web::empty(500)
        }
    }
}

fn with_version(backend: &dyn Backend, id: &str, reply: Reply) -> Reply {
    match backend.file(id) {
        Ok(file) => reply.with_header(ITEM_VERSION, file.etag),
        Err(_) => reply,
    }
}

#[cfg(test)]
mod tests {
    use crate::backend::fake::{Fake, FILE};
    use crate::backend::Reply;
    use crate::wopi::tests::call;

    fn post(fake: &Fake, headers: &[(&str, &str)]) -> Reply {
        call(fake, "POST", FILE, headers)
    }

    fn lock_header(reply: &Reply) -> Option<&str> {
        reply.header("X-WOPI-Lock")
    }

    #[test]
    fn lock_and_unlock() {
        let fake = Fake::default();
        let reply = post(&fake, &[("X-WOPI-Override", "LOCK"), ("X-WOPI-Lock", "L1")]);
        assert_eq!(reply.status, 200);
        assert_eq!(reply.header("X-WOPI-ItemVersion"), Some("etag-1"));
        assert_eq!(fake.lock_id().as_deref(), Some("L1"));

        // The same lock again renews it.
        assert_eq!(post(&fake, &[("X-WOPI-Override", "LOCK"), ("X-WOPI-Lock", "L1")]).status, 200);
        // Another one is refused, with the one held.
        let other = post(&fake, &[("X-WOPI-Override", "LOCK"), ("X-WOPI-Lock", "L2")]);
        assert_eq!(other.status, 409);
        assert_eq!(lock_header(&other), Some("L1"));

        let get = post(&fake, &[("X-WOPI-Override", "GET_LOCK")]);
        assert_eq!((get.status, lock_header(&get)), (200, Some("L1")));

        let wrong = post(&fake, &[("X-WOPI-Override", "UNLOCK"), ("X-WOPI-Lock", "L2")]);
        assert_eq!((wrong.status, lock_header(&wrong)), (409, Some("L1")));
        assert_eq!(post(&fake, &[("X-WOPI-Override", "UNLOCK"), ("X-WOPI-Lock", "L1")]).status, 200);
        assert_eq!(fake.lock_id(), None);

        let none = post(&fake, &[("X-WOPI-Override", "GET_LOCK")]);
        assert_eq!((none.status, lock_header(&none)), (200, Some("")));
        // Unlocking an unlocked file: 409 with an empty lock.
        let unlocked = post(&fake, &[("X-WOPI-Override", "UNLOCK"), ("X-WOPI-Lock", "L1")]);
        assert_eq!((unlocked.status, lock_header(&unlocked)), (409, Some("")));
    }

    #[test]
    fn refresh() {
        let fake = Fake::default();
        // Nothing to refresh.
        let unlocked = post(&fake, &[("X-WOPI-Override", "REFRESH_LOCK"), ("X-WOPI-Lock", "L1")]);
        assert_eq!((unlocked.status, lock_header(&unlocked)), (409, Some("")));
        assert_eq!(fake.lock_id(), None, "refreshing doesn't lock");

        fake.lock_as(crate::APP_ID, "L1");
        assert_eq!(post(&fake, &[("X-WOPI-Override", "REFRESH_LOCK"), ("X-WOPI-Lock", "L1")]).status, 200);
        let other = post(&fake, &[("X-WOPI-Override", "REFRESH_LOCK"), ("X-WOPI-Lock", "L2")]);
        assert_eq!((other.status, lock_header(&other)), (409, Some("L1")));
    }

    #[test]
    fn unlock_and_relock() {
        let fake = Fake::default();
        let relock = [("X-WOPI-Override", "LOCK"), ("X-WOPI-Lock", "L2"), ("X-WOPI-OldLock", "L1")];
        let unlocked = post(&fake, &relock);
        assert_eq!((unlocked.status, lock_header(&unlocked)), (409, Some("")));

        fake.lock_as(crate::APP_ID, "L1");
        assert_eq!(post(&fake, &relock).status, 200);
        assert_eq!(fake.lock_id().as_deref(), Some("L2"));

        // The old lock no longer matches.
        let stale = post(&fake, &[("X-WOPI-Override", "LOCK"), ("X-WOPI-Lock", "L3"), ("X-WOPI-OldLock", "L1")]);
        assert_eq!((stale.status, lock_header(&stale)), (409, Some("L2")));
    }

    #[test]
    fn another_apps_lock() {
        let fake = Fake::default();
        fake.lock_as("rc-notes", "N1");
        for headers in [
            vec![("X-WOPI-Override", "LOCK"), ("X-WOPI-Lock", "N1")],
            vec![("X-WOPI-Override", "REFRESH_LOCK"), ("X-WOPI-Lock", "N1")],
            vec![("X-WOPI-Override", "UNLOCK"), ("X-WOPI-Lock", "N1")],
        ] {
            let reply = post(&fake, &headers);
            assert_eq!(reply.status, 409, "{headers:?}");
            assert_eq!(lock_header(&reply), Some(""));
            assert!(reply.header("X-WOPI-LockFailureReason").unwrap().contains("rc-notes"));
        }
        let get = post(&fake, &[("X-WOPI-Override", "GET_LOCK")]);
        assert_eq!((get.status, lock_header(&get)), (200, Some("")));
        assert_eq!(fake.lock_id().as_deref(), Some("N1"));
    }

    #[test]
    fn needs_a_lock_id() {
        let fake = Fake::default();
        for operation in ["LOCK", "REFRESH_LOCK", "UNLOCK"] {
            assert_eq!(post(&fake, &[("X-WOPI-Override", operation)]).status, 400, "{operation}");
            assert_eq!(post(&fake, &[("X-WOPI-Override", operation), ("X-WOPI-Lock", " ")]).status, 400);
        }
    }

    #[test]
    fn read_only_file() {
        let fake = Fake { writable: false, ..Fake::default() };
        let reply = post(&fake, &[("X-WOPI-Override", "LOCK"), ("X-WOPI-Lock", "L1")]);
        assert_eq!(reply.status, 409);
        assert_eq!(fake.lock_id(), None);
    }

    #[test]
    fn missing_file() {
        let fake = Fake::default();
        fake.files.borrow_mut().clear();
        assert_eq!(post(&fake, &[("X-WOPI-Override", "GET_LOCK")]).status, 404);
        assert_eq!(post(&fake, &[("X-WOPI-Override", "LOCK"), ("X-WOPI-Lock", "L1")]).status, 404);
    }
}
