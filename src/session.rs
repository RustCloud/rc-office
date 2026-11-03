//! The page's own routes: `/session` opens a document in Collabora, `/status`
//! tells how the plugin gets on with it.

use serde_json::json;

use crate::backend::{Backend, FileError, Reply, Request, TokenError};
use crate::discovery::{self, Discovery};
use crate::{time, web, APP_ID};

/// How long Collabora may work with a document it was given: a working day.
/// The server allows 12 hours at most; later Collabora asks to reload.
pub const TOKEN_SECONDS: u32 = 10 * 3600;

/// The plugin's server settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    /// Collabora, as browsers and the server reach it.
    pub office_url: Option<String>,
    /// The cloud as Collabora reaches it, if not at the panel's address.
    pub wopi_base_url: Option<String>,
}

impl Settings {
    pub fn of(backend: &dyn Backend) -> Self {
        let values = backend.settings();
        Settings {
            office_url: web::base_url(values["office_url"].as_str()),
            wopi_base_url: web::base_url(values["wopi_base_url"].as_str()),
        }
    }
}

/// The panel's origin a page opened a document from: Collabora talks to it
/// (`PostMessageOrigin`). Kept in the user's AppData, per file.
pub fn origin_of(backend: &dyn Backend, file: &str) -> Option<String> {
    let bytes = backend.appdata_read(&session_path(file))?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    value["origin"].as_str().map(str::to_owned)
}

fn session_path(file: &str) -> String {
    format!("sessions/{file}.json")
}

/// `GET /session?file=<id>&origin=<panel>&lang=<en|ru>&theme=<light|dark>[&write=0][&refresh=1]`:
/// the editor's address for the file and a token for it.
pub fn open(backend: &dyn Backend, request: &Request) -> Reply {
    let query = &request.query;
    let id = web::query_param(query, "file");
    if !web::valid_id(&id) {
        return web::error(400, "bad-request", "No file");
    }
    let origin = web::query_param(query, "origin");
    if web::origin(&origin).as_deref() != Some(origin.as_str()) {
        return web::error(400, "bad-request", "No origin of the panel");
    }
    let settings = Settings::of(backend);
    let Some(office_url) = settings.office_url else {
        return web::error(503, "no-office-url", "The administrator has not set the address of Collabora Online");
    };
    let file = match backend.file(&id) {
        Ok(file) => file,
        Err(FileError::NotFound) => return web::error(404, "not-found", "The file is not open for the app"),
        Err(err) => return web::error(500, "failed", &format!("{err:?}")),
    };
    let discovery = match discovery::get(backend, &office_url, web::query_param(query, "refresh") == "1") {
        Ok(discovery) => discovery,
        Err(err) => return office_error(&err),
    };
    let mut write = web::query_param(query, "write") != "0";
    let Some(action) = discovery.find(&file.name, file.mime_type.as_deref(), write) else {
        return web::error(415, "unsupported", "Collabora Online does not open files of this type");
    };
    write &= action.name == "edit";
    // Another app edits it: Collabora could not save, so it only views.
    let locked_by = match backend.get_lock(&id) {
        Ok(Some(held)) if held.app_id != APP_ID => Some(held.app_id),
        _ => None,
    };
    write &= locked_by.is_none();
    let token = match backend.issue_token(&id, write, TOKEN_SECONDS) {
        Err(TokenError::ReadOnly) if write => {
            write = false;
            backend.issue_token(&id, false, TOKEN_SECONDS)
        }
        token => token,
    };
    let token = match token {
        Ok(token) => token,
        Err(TokenError::NotFound) => return web::error(404, "not-found", "The file is not open for the app"),
        Err(err) => return web::error(500, "failed", &format!("token: {err:?}")),
    };
    let session = json!({ "origin": origin, "updated_ms": backend.now_ms() });
    if let Err(why) = backend.appdata_write(&session_path(&id), session.to_string().as_bytes()) {
        backend.warn(&format!("session not kept: {why}"));
    }
    let base = settings.wopi_base_url.unwrap_or(origin);
    let wopi_src = format!("{base}/app/{APP_ID}/wopi/files/{id}");
    let lang = match web::query_param(query, "lang").as_str() {
        "" => String::new(),
        lang => lang.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').take(16).collect(),
    };
    let dark = web::query_param(query, "theme") == "dark";
    let url = discovery::editor_url(&action.urlsrc, &office_url, &wopi_src, &lang, dark);
    web::json(
        200,
        json!({
            "url": url,
            "token": token.token,
            // WOPI's access_token_ttl: when it runs out, ms since 1970.
            "ttl": time::parse_ms(&token.expires_at).unwrap_or(0),
            "write": write,
            "action": action.name,
            "name": file.name,
            "locked_by": locked_by,
        }),
    )
}

/// `GET /status`: whether Collabora is set and answers, its version and what it edits.
pub fn status(backend: &dyn Backend) -> Reply {
    let settings = Settings::of(backend);
    let Some(office_url) = settings.office_url.clone() else {
        return web::json(200, json!({ "configured": false }));
    };
    let mut status = json!({
        "configured": true,
        "office_url": office_url,
        "wopi_base_url": settings.wopi_base_url,
    });
    // A common slip: the address of Collabora itself instead of the cloud's.
    let office_origin = web::origin(&office_url);
    if settings.wopi_base_url.as_deref().and_then(web::origin).is_some_and(|base| Some(base) == office_origin) {
        status["wopi_base_is_office"] = json!(true);
    }
    match discovery::get(backend, &office_url, true) {
        Ok(discovery) => {
            status["reachable"] = json!(true);
            status["editable"] = json!(Discovery::editable(&discovery));
        }
        Err(err) => {
            status["reachable"] = json!(false);
            status["error"] = json!(err.to_string());
        }
    }
    if let Ok(answer) = backend.fetch(&format!("{office_url}/hosting/capabilities")) {
        if let Ok(capabilities) = serde_json::from_slice::<serde_json::Value>(&answer.body) {
            status["product"] = capabilities["productName"].clone();
            status["version"] = capabilities["productVersion"].clone();
        }
    }
    web::json(200, status)
}

fn office_error(err: &discovery::Error) -> Reply {
    match err {
        discovery::Error::Unreachable(_) => web::error(502, "office-unreachable", &err.to_string()),
        discovery::Error::Invalid(_) => web::error(502, "office-invalid", &err.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::fake::{Fake, FILE};
    use crate::backend::Fetched;

    const DISCOVERY: &str = "http://office.test:9980/hosting/discovery";

    fn fake() -> Fake {
        let fake = Fake { token: None, ..Fake::default() };
        let xml = include_str!("../tests/discovery.xml");
        fake.web.borrow_mut().insert(DISCOVERY.into(), Ok(Fetched { status: 200, body: xml.into() }));
        fake
    }

    fn open_with(fake: &Fake, query: &str) -> (u16, serde_json::Value) {
        let request =
            Request { method: "GET".into(), path: "/session".into(), query: query.into(), ..Request::default() };
        let reply = open(fake, &request);
        (reply.status, serde_json::from_slice(&reply.body).unwrap())
    }

    fn query() -> String {
        format!("file={FILE}&origin=https%3A%2F%2Fcloud.example.com&lang=ru&theme=dark")
    }

    #[test]
    fn opens_a_document() {
        let fake = fake();
        let (status, body) = open_with(&fake, &query());
        assert_eq!(status, 200, "{body}");
        assert_eq!(
            body["url"],
            format!(
                "http://office.test:9980/browser/0c4e5b1/cool.html?WOPISrc=https%3A%2F%2Fcloud.example.com\
                 %2Fapp%2Frc-office%2Fwopi%2Ffiles%2F{FILE}&lang=ru&ui_theme=dark"
            )
        );
        assert_eq!(body["token"], format!("token-{FILE}-true-36000"));
        assert_eq!(body["ttl"], 1_793_736_000_000_i64);
        assert_eq!(body["write"], true);
        assert_eq!(origin_of(&fake, FILE).as_deref(), Some("https://cloud.example.com"));
    }

    #[test]
    fn wopi_base_from_settings() {
        let mut fake = fake();
        fake.settings = json!({ "office_url": "http://office.test:9980/", "wopi_base_url": "http://rustcloud:8080/" });
        let (_, body) = open_with(&fake, &query());
        assert!(body["url"].as_str().unwrap().contains("WOPISrc=http%3A%2F%2Frustcloud%3A8080%2Fapp%2Frc-office"));
        // Collabora still talks to the panel.
        assert_eq!(origin_of(&fake, FILE).as_deref(), Some("https://cloud.example.com"));
    }

    #[test]
    fn warns_when_wopi_base_is_collabora() {
        let mut fake = fake();
        fake.settings = json!({ "office_url": "http://office.test:9980/", "wopi_base_url": "http://OFFICE.test:9980" });
        let body: serde_json::Value = serde_json::from_slice(&status(&fake).body).unwrap();
        assert_eq!(body["wopi_base_is_office"], true);
        fake.settings = json!({ "office_url": "http://office.test:9980/", "wopi_base_url": "http://rustcloud:8080" });
        let body: serde_json::Value = serde_json::from_slice(&status(&fake).body).unwrap();
        assert!(body.get("wopi_base_is_office").is_none());
    }

    #[test]
    fn read_only() {
        let locked = Fake { writable: false, ..fake() };
        let (status, body) = open_with(&locked, &query());
        assert_eq!(status, 200);
        assert_eq!(body["write"], false);
        assert_eq!(body["token"], format!("token-{FILE}-false-36000"));

        // A PDF only views: no token to change it.
        let pdf = fake();
        pdf.files.borrow_mut().get_mut(FILE).unwrap().name = "Scan.pdf".into();
        let (_, body) = open_with(&pdf, &query());
        assert_eq!((body["write"].clone(), body["action"].clone()), (json!(false), json!("view")));
    }

    #[test]
    fn locked_by_another_app() {
        let other = fake();
        other.lock_as("rc-notes", "N1");
        let (status, body) = open_with(&other, &query());
        assert_eq!(status, 200);
        assert_eq!(body["write"], false);
        assert_eq!(body["locked_by"], "rc-notes");
        // Its own lock (another tab, a reload): editing as usual.
        let own = fake();
        own.lock_as(crate::APP_ID, "L1");
        assert_eq!(open_with(&own, &query()).1["write"], true);
    }

    #[test]
    fn explains_failures() {
        let unset = Fake { settings: json!({}), ..fake() };
        assert_eq!(open_with(&unset, &query()).1["error"], "no-office-url");

        let down = Fake { token: None, ..Fake::default() };
        assert_eq!(open_with(&down, &query()), (502, open_with(&down, &query()).1));
        assert_eq!(open_with(&down, &query()).1["error"], "office-unreachable");

        let picture = fake();
        let mut files = picture.files.borrow_mut();
        let file = files.get_mut(FILE).unwrap();
        file.name = "photo.jpg".into();
        file.mime_type = Some("image/jpeg".into());
        drop(files);
        assert_eq!(open_with(&picture, &query()).1["error"], "unsupported");

        let missing = fake();
        assert_eq!(open_with(&missing, "file=0000&origin=https%3A%2F%2Fa.b").0, 404);
        assert_eq!(open_with(&missing, &format!("file={FILE}&origin=javascript%3Ax")).0, 400);
        assert_eq!(open_with(&missing, &format!("file={FILE}&origin=https%3A%2F%2Fa.b%2Fpath")).0, 400);
        assert_eq!(open_with(&missing, "origin=https%3A%2F%2Fa.b").0, 400);
    }

    #[test]
    fn reports_status() {
        let fake = fake();
        fake.web.borrow_mut().insert(
            "http://office.test:9980/hosting/capabilities".into(),
            Ok(Fetched {
                status: 200,
                body: br#"{"productName":"Collabora Online Development Edition","productVersion":"26.04.5.2"}"#
                    .to_vec(),
            }),
        );
        let body: serde_json::Value = serde_json::from_slice(&status(&fake).body).unwrap();
        assert_eq!(body["configured"], true);
        assert_eq!(body["reachable"], true);
        assert_eq!(body["version"], "26.04.5.2");
        assert!(body["editable"].as_array().unwrap().contains(&json!("docx")));

        let down = Fake::default();
        let body: serde_json::Value = serde_json::from_slice(&status(&down).body).unwrap();
        assert_eq!(body["reachable"], false);
        assert!(body["error"].as_str().unwrap().contains("does not answer"));

        let unset = Fake { settings: json!({ "office_url": "" }), ..Fake::default() };
        let body: serde_json::Value = serde_json::from_slice(&status(&unset).body).unwrap();
        assert_eq!(body["configured"], false);
    }
}
