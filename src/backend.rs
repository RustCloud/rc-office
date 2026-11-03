//! What the plugin needs of the server, behind a trait: the WOPI logic runs
//! on [`Backend`], the plugin on the server's imports ([`crate::host`]), the
//! tests on a fake one ([`fake`]).

/// The logged-in user (or the user a token was given for).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct User {
    /// UUID.
    pub id: String,
    pub username: String,
    pub display_name: String,
}

/// A request to the plugin's routes; header names in lower case.
#[derive(Debug, Clone, Default)]
pub struct Request {
    pub method: String,
    /// Below `/app/rc-office`, starting with `/`.
    pub path: String,
    pub query: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Request {
    /// The value of a header, by its lower-case name.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(key, _)| key.eq_ignore_ascii_case(name)).map(|(_, value)| value.as_str())
    }
}

/// The plugin's answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reply {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Reply {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(key, _)| key.eq_ignore_ascii_case(name)).map(|(_, value)| value.as_str())
    }

    pub fn with_header(mut self, name: &str, value: impl Into<String>) -> Self {
        self.headers.push((name.to_owned(), value.into()));
        self
    }
}

/// A file of the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct File {
    pub id: String,
    pub name: String,
    pub size: u64,
    pub mime_type: Option<String>,
    /// Changes with the content.
    pub etag: String,
    /// RFC 3339.
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileError {
    NotFound,
    ReadOnly,
    NoSpace,
    TooLarge,
    /// The file changed since (`if-match`), or another app holds a lock.
    Conflict,
    Other(String),
}

/// A lock on a file, taken by an app.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lock {
    pub lock_id: String,
    pub app_id: String,
    pub user_id: String,
    pub expires_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LockError {
    NotFound,
    ReadOnly,
    /// Someone else holds this other lock.
    Locked(Lock),
    Other(String),
}

/// What the token of a request allows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scope {
    pub file: String,
    pub write: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub token: String,
    /// RFC 3339.
    pub expires_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenError {
    NoUser,
    NotFound,
    ReadOnly,
    Other(String),
}

/// An answer of another server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fetched {
    pub status: u16,
    pub body: Vec<u8>,
}

pub trait Backend {
    /// The scope of the token this request came with; none for the user's own requests.
    fn token(&self) -> Option<Scope>;
    fn file(&self, id: &str) -> Result<File, FileError>;
    /// Saves the body of the request into the file, if it still has `if_match`.
    fn write_body(&self, id: &str, if_match: Option<&str>) -> Result<File, FileError>;
    /// Takes or renews the lock.
    fn lock(&self, id: &str, lock_id: &str, seconds: u32) -> Result<(), LockError>;
    fn unlock(&self, id: &str, lock_id: &str) -> Result<(), LockError>;
    fn get_lock(&self, id: &str) -> Result<Option<Lock>, FileError>;
    fn issue_token(&self, file: &str, write: bool, seconds: u32) -> Result<Token, TokenError>;
    /// A file of the user's AppData; none if missing or unreadable.
    fn appdata_read(&self, path: &str) -> Option<Vec<u8>>;
    fn appdata_write(&self, path: &str, data: &[u8]) -> Result<(), String>;
    /// A file of the plugin's storage (shared by all users).
    fn storage_read(&self, path: &str) -> Option<Vec<u8>>;
    fn storage_write(&self, path: &str, data: &[u8]) -> Result<(), String>;
    /// `GET url`.
    fn fetch(&self, url: &str) -> Result<Fetched, String>;
    /// The server settings of the plugin, a JSON object.
    fn settings(&self) -> serde_json::Value;
    /// Milliseconds since 1970.
    fn now_ms(&self) -> i64;
    fn warn(&self, message: &str);
}

/// A server in memory, for tests.
#[cfg(test)]
pub mod fake {
    use std::cell::RefCell;
    use std::collections::HashMap;

    use super::*;

    pub const APP: &str = crate::APP_ID;

    pub struct Fake {
        pub token: Option<Scope>,
        pub files: RefCell<HashMap<String, File>>,
        pub locks: RefCell<HashMap<String, Lock>>,
        /// What the request's body would write; none: no body.
        pub body: Vec<u8>,
        pub contents: RefCell<HashMap<String, Vec<u8>>>,
        pub appdata: RefCell<HashMap<String, Vec<u8>>>,
        pub storage: RefCell<HashMap<String, Vec<u8>>>,
        /// URL → answer.
        pub web: RefCell<HashMap<String, Result<Fetched, String>>>,
        pub fetches: RefCell<Vec<String>>,
        pub settings: serde_json::Value,
        pub now_ms: i64,
        /// Changes made to a file behind the plugin's back before `write_body` runs.
        pub race: RefCell<bool>,
        pub writable: bool,
        /// What `write_body` fails with, if anything.
        pub write_error: Option<FileError>,
        pub writes: RefCell<u32>,
    }

    pub const FILE: &str = "0b9e2f3c-1d2e-4f5a-8b7c-112233445566";

    impl Default for Fake {
        fn default() -> Self {
            let file = File {
                id: FILE.into(),
                name: "Report.docx".into(),
                size: 1200,
                mime_type: Some("application/vnd.openxmlformats-officedocument.wordprocessingml.document".into()),
                etag: "etag-1".into(),
                updated_at: "2026-11-03T10:00:00.123456Z".into(),
            };
            Self {
                token: Some(Scope { file: FILE.into(), write: true }),
                files: RefCell::new(HashMap::from([(FILE.to_owned(), file)])),
                locks: RefCell::default(),
                body: b"new content".to_vec(),
                contents: RefCell::default(),
                appdata: RefCell::default(),
                storage: RefCell::default(),
                web: RefCell::default(),
                fetches: RefCell::default(),
                settings: serde_json::json!({ "office_url": "http://office.test:9980" }),
                now_ms: 1_793_000_000_000,
                race: RefCell::new(false),
                writable: true,
                write_error: None,
                writes: RefCell::new(0),
            }
        }
    }

    impl Fake {
        pub fn lock_as(&self, app: &str, lock_id: &str) {
            self.locks.borrow_mut().insert(
                FILE.into(),
                Lock {
                    lock_id: lock_id.into(),
                    app_id: app.into(),
                    user_id: "u1".into(),
                    expires_at: "2026-11-03T11:00:00Z".into(),
                },
            );
        }

        pub fn lock_id(&self) -> Option<String> {
            self.locks.borrow().get(FILE).map(|lock| lock.lock_id.clone())
        }

        pub fn set_size(&self, size: u64) {
            self.files.borrow_mut().get_mut(FILE).unwrap().size = size;
        }
    }

    impl Backend for Fake {
        fn token(&self) -> Option<Scope> {
            self.token.clone()
        }

        fn file(&self, id: &str) -> Result<File, FileError> {
            self.files.borrow().get(id).cloned().ok_or(FileError::NotFound)
        }

        fn write_body(&self, id: &str, if_match: Option<&str>) -> Result<File, FileError> {
            if !self.writable {
                return Err(FileError::ReadOnly);
            }
            if let Some(err) = &self.write_error {
                return Err(err.clone());
            }
            if self.race.replace(false) {
                let mut files = self.files.borrow_mut();
                let file = files.get_mut(id).ok_or(FileError::NotFound)?;
                file.etag = "etag-raced".into();
            }
            if let Some(lock) = self.locks.borrow().get(id) {
                if lock.app_id != APP {
                    return Err(FileError::Conflict);
                }
            }
            let mut files = self.files.borrow_mut();
            let file = files.get_mut(id).ok_or(FileError::NotFound)?;
            if if_match.is_some_and(|etag| etag != file.etag) {
                return Err(FileError::Conflict);
            }
            *self.writes.borrow_mut() += 1;
            file.size = self.body.len() as u64;
            file.etag = format!("etag-{}", *self.writes.borrow() + 1);
            file.updated_at = "2026-11-03T10:05:00.5Z".into();
            self.contents.borrow_mut().insert(id.into(), self.body.clone());
            Ok(file.clone())
        }

        fn lock(&self, id: &str, lock_id: &str, _seconds: u32) -> Result<(), LockError> {
            if !self.files.borrow().contains_key(id) {
                return Err(LockError::NotFound);
            }
            if !self.writable {
                return Err(LockError::ReadOnly);
            }
            let mut locks = self.locks.borrow_mut();
            if let Some(held) = locks.get(id) {
                if held.app_id != APP || held.lock_id != lock_id {
                    return Err(LockError::Locked(held.clone()));
                }
            }
            locks.insert(
                id.into(),
                Lock {
                    lock_id: lock_id.into(),
                    app_id: APP.into(),
                    user_id: "u1".into(),
                    expires_at: "2026-11-03T10:30:00Z".into(),
                },
            );
            Ok(())
        }

        fn unlock(&self, id: &str, lock_id: &str) -> Result<(), LockError> {
            let mut locks = self.locks.borrow_mut();
            match locks.get(id) {
                Some(held) if held.app_id != APP || held.lock_id != lock_id => Err(LockError::Locked(held.clone())),
                Some(_) => {
                    locks.remove(id);
                    Ok(())
                }
                None => Ok(()),
            }
        }

        fn get_lock(&self, id: &str) -> Result<Option<Lock>, FileError> {
            if !self.files.borrow().contains_key(id) {
                return Err(FileError::NotFound);
            }
            Ok(self.locks.borrow().get(id).cloned())
        }

        fn issue_token(&self, file: &str, write: bool, seconds: u32) -> Result<Token, TokenError> {
            if self.token.is_some() {
                return Err(TokenError::NoUser);
            }
            if !self.files.borrow().contains_key(file) {
                return Err(TokenError::NotFound);
            }
            if write && !self.writable {
                return Err(TokenError::ReadOnly);
            }
            Ok(Token { token: format!("token-{file}-{write}-{seconds}"), expires_at: "2026-11-03T20:00:00Z".into() })
        }

        fn appdata_read(&self, path: &str) -> Option<Vec<u8>> {
            self.appdata.borrow().get(path).cloned()
        }

        fn appdata_write(&self, path: &str, data: &[u8]) -> Result<(), String> {
            self.appdata.borrow_mut().insert(path.into(), data.to_vec());
            Ok(())
        }

        fn storage_read(&self, path: &str) -> Option<Vec<u8>> {
            self.storage.borrow().get(path).cloned()
        }

        fn storage_write(&self, path: &str, data: &[u8]) -> Result<(), String> {
            self.storage.borrow_mut().insert(path.into(), data.to_vec());
            Ok(())
        }

        fn fetch(&self, url: &str) -> Result<Fetched, String> {
            self.fetches.borrow_mut().push(url.into());
            self.web.borrow().get(url).cloned().unwrap_or_else(|| Err("connection refused".into()))
        }

        fn settings(&self) -> serde_json::Value {
            self.settings.clone()
        }

        fn now_ms(&self) -> i64 {
            self.now_ms
        }

        fn warn(&self, _message: &str) {}
    }
}
