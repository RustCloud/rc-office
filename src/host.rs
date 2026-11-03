//! [`Backend`] over the server's imports.

use crate::backend::{self, Backend, Fetched, File, FileError, Lock, LockError, Scope, Token, TokenError};
use crate::rustcloud::app::{appdata, files, http, log, proxy, settings, storage, tokens};

pub struct Host;

fn file(node: files::Node) -> File {
    File {
        id: node.id,
        name: node.name,
        size: node.size,
        mime_type: node.mime_type,
        etag: node.etag.unwrap_or_default(),
        updated_at: node.updated_at,
    }
}

fn file_error(err: files::Error) -> FileError {
    match err {
        files::Error::NotFound | files::Error::NoUser => FileError::NotFound,
        files::Error::ReadOnly => FileError::ReadOnly,
        files::Error::NoSpace => FileError::NoSpace,
        files::Error::TooLarge => FileError::TooLarge,
        files::Error::Conflict => FileError::Conflict,
        files::Error::InvalidName => FileError::Other("invalid name".into()),
        files::Error::Other(why) => FileError::Other(why),
    }
}

fn lock(lock: files::FileLock) -> Lock {
    Lock { lock_id: lock.lock_id, app_id: lock.app_id, user_id: lock.user_id, expires_at: lock.expires_at }
}

fn lock_error(err: files::LockError) -> LockError {
    match err {
        files::LockError::NoUser | files::LockError::NotFound => LockError::NotFound,
        files::LockError::ReadOnly => LockError::ReadOnly,
        files::LockError::Locked(held) => LockError::Locked(lock(held)),
        files::LockError::Other(why) => LockError::Other(why),
    }
}

impl Backend for Host {
    fn token(&self) -> Option<Scope> {
        tokens::current().map(|scope| Scope { file: scope.file, write: scope.write })
    }

    fn file(&self, id: &str) -> Result<File, FileError> {
        files::get(id).map(file).map_err(file_error)
    }

    fn write_body(&self, id: &str, if_match: Option<&str>) -> Result<File, FileError> {
        files::write_request_body(id, if_match).map(file).map_err(file_error)
    }

    fn lock(&self, id: &str, lock_id: &str, seconds: u32) -> Result<(), LockError> {
        files::lock(id, lock_id, seconds).map(|_| ()).map_err(lock_error)
    }

    fn unlock(&self, id: &str, lock_id: &str) -> Result<(), LockError> {
        files::unlock(id, lock_id).map_err(lock_error)
    }

    fn get_lock(&self, id: &str) -> Result<Option<Lock>, FileError> {
        files::get_lock(id).map(|held| held.map(lock)).map_err(file_error)
    }

    fn issue_token(&self, file: &str, write: bool, seconds: u32) -> Result<Token, TokenError> {
        tokens::issue(file, write, seconds)
            .map(|token| Token { token: token.token, expires_at: token.expires_at })
            .map_err(|err| match err {
                tokens::Error::NoUser => TokenError::NoUser,
                tokens::Error::NotFound => TokenError::NotFound,
                tokens::Error::ReadOnly => TokenError::ReadOnly,
                tokens::Error::Other(why) => TokenError::Other(why),
            })
    }

    fn appdata_read(&self, path: &str) -> Option<Vec<u8>> {
        appdata::read(path).ok()
    }

    fn appdata_write(&self, path: &str, data: &[u8]) -> Result<(), String> {
        appdata::write(path, data).map(|_| ()).map_err(|err| format!("{err:?}"))
    }

    fn storage_read(&self, path: &str) -> Option<Vec<u8>> {
        let size = storage::stat(path).ok()?.size;
        let mut bytes = Vec::with_capacity(size as usize);
        while (bytes.len() as u64) < size {
            let part = storage::read(path, bytes.len() as u64, size - bytes.len() as u64).ok()?;
            if part.is_empty() {
                break;
            }
            bytes.extend(part);
        }
        Some(bytes)
    }

    fn storage_write(&self, path: &str, data: &[u8]) -> Result<(), String> {
        storage::write(path, data).map(|_| ()).map_err(|err| format!("{err:?}"))
    }

    fn fetch(&self, url: &str) -> Result<Fetched, String> {
        let request = http::Request { method: "GET".into(), url: url.into(), headers: Vec::new(), body: Vec::new() };
        match http::fetch(&request) {
            Ok(answer) => Ok(Fetched { status: answer.status, body: answer.body }),
            Err(http::Error::NotAllowed) => Err("the address is not allowed (office_url?)".into()),
            Err(http::Error::InvalidRequest) => Err("not a valid address".into()),
            Err(http::Error::TooLarge) => Err("the answer is too large".into()),
            Err(http::Error::Timeout) => Err("no answer in time".into()),
            Err(http::Error::Failed(why)) => Err(why),
        }
    }

    fn settings(&self) -> serde_json::Value {
        serde_json::from_str(&settings::values()).unwrap_or_default()
    }

    fn self_url(&self) -> Option<String> {
        proxy::self_url()
    }

    fn now_ms(&self) -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_millis() as i64)
            .unwrap_or(0)
    }

    fn warn(&self, message: &str) {
        log::warn(message);
    }
}

impl From<crate::User> for backend::User {
    fn from(user: crate::User) -> Self {
        backend::User { id: user.id, username: user.username, display_name: user.display_name }
    }
}
