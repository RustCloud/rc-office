//! The plugin's own routes: `/session` opens a document in Collabora, `/status`
//! tells the administrator how the plugin gets on with it.

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
    /// Collabora as the server reaches it (`http://collabora:9980`); browsers
    /// reach it through the server's address (`[[proxy]]`).
    pub office_url: Option<String>,
    /// The cloud as Collabora reaches it, if the server can't tell itself
    /// (`proxy.self-url`).
    pub wopi_base_url: Option<String>,
    /// The classic toolbar instead of tabs (the notebookbar).
    pub compact_ui: bool,
    /// A theme of Collabora's own (`theme` field), e.g. `nextcloud`.
    pub theme: Option<String>,
}

impl Settings {
    pub fn of(backend: &dyn Backend) -> Self {
        let values = backend.settings();
        Settings {
            office_url: web::base_url(values["office_url"].as_str()),
            wopi_base_url: web::base_url(values["wopi_base_url"].as_str()),
            compact_ui: values["compact_ui"].as_bool().unwrap_or(false),
            theme: values["theme"]
                .as_str()
                .map(str::trim)
                .filter(|name| {
                    !name.is_empty()
                        && name.len() <= 32
                        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
                })
                .map(str::to_owned),
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
    let Some(office_url) = settings.office_url.clone() else {
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
    let base = wopi_base(backend, &settings, &origin);
    let wopi_src = format!("{base}/app/{APP_ID}/wopi/files/{id}");
    let lang = match web::query_param(query, "lang").as_str() {
        "" => String::new(),
        lang => lang.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').take(16).collect(),
    };
    let dark = web::query_param(query, "theme") == "dark";
    let url = discovery::editor_url(&action.urlsrc, &origin, &wopi_src, &lang);
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
            "fields": fields(&settings, dark),
        }),
    )
}

/// What the page posts to Collabora with the token: how the editor looks.
/// The page adds `css_variables`, the panel's colours.
fn fields(settings: &Settings, dark: bool) -> serde_json::Value {
    let mode = if settings.compact_ui { "compact" } else { "tabbed" };
    let mut fields = json!({
        // The user's own choices in Collabora's settings win (SavedUIState).
        "ui_defaults": format!("UIMode={mode};SavedUIState=true"),
        "ui_theme": if dark { "dark" } else { "light" },
    });
    if let Some(theme) = &settings.theme {
        fields["theme"] = json!(theme);
    }
    fields
}

/// Where Collabora calls the plugin back (WOPI): the setting if the
/// administrator gave one, else the server's own address on its connection to
/// Collabora, else the panel's.
fn wopi_base(backend: &dyn Backend, settings: &Settings, origin: &str) -> String {
    settings
        .wopi_base_url
        .clone()
        .or_else(|| web::base_url(backend.self_url().as_deref()))
        .unwrap_or_else(|| origin.to_owned())
}

/// One line of the report for the administrator's plugin dialog (`status`
/// in the manifest): a level and the text by language.
fn line(level: &str, en: String, ru: String) -> serde_json::Value {
    json!({ "level": level, "text": { "en": en, "ru": ru } })
}

/// `GET /status`: the report the plugin dialog (Administration → Plugins →
/// Office → Status) shows: whether Collabora is set and answers, its version,
/// the addresses, the theme and what it edits. A JSON list of lines.
pub fn status(backend: &dyn Backend) -> Reply {
    web::json(200, serde_json::Value::Array(report(backend)))
}

fn report(backend: &dyn Backend) -> Vec<serde_json::Value> {
    let settings = Settings::of(backend);
    let Some(office_url) = settings.office_url.clone() else {
        return vec![line(
            "error",
            "The address of Collabora Online is not set (Settings tab).".into(),
            "Не задан адрес Collabora Online (вкладка «Настройки»).".into(),
        )];
    };
    let mut lines = Vec::new();
    let discovery = discovery::get(backend, &office_url, true);
    match &discovery {
        Ok(_) => {
            let product = backend
                .fetch(&format!("{office_url}/hosting/capabilities"))
                .ok()
                .and_then(|answer| serde_json::from_slice::<serde_json::Value>(&answer.body).ok())
                .map(|caps| {
                    let name = caps["productName"].as_str().unwrap_or("Collabora Online").to_owned();
                    match caps["productVersion"].as_str() {
                        Some(version) => format!("{name} {version}"),
                        None => name,
                    }
                })
                .unwrap_or_else(|| "Collabora Online".into());
            lines.push(line(
                "ok",
                format!("{product} answers at {office_url}"),
                format!("{product} отвечает по адресу {office_url}"),
            ));
        }
        Err(err) => lines.push(line(
            "error",
            format!("Collabora Online does not answer at {office_url}: {err}"),
            format!("Collabora Online не отвечает по адресу {office_url}: {err}"),
        )),
    }

    // After discovery: the server learns its address on a connection to Collabora.
    let self_url = web::base_url(backend.self_url().as_deref());
    match (&settings.wopi_base_url, &self_url) {
        (Some(base), _) => lines.push(line(
            "info",
            format!("Collabora calls the cloud at {base} (from the settings)"),
            format!("Collabora обращается к облаку по адресу {base} (из настроек)"),
        )),
        (None, Some(base)) => lines.push(line(
            "info",
            format!("Collabora calls the cloud at {base} (found by the server)"),
            format!("Collabora обращается к облаку по адресу {base} (определил сервер)"),
        )),
        (None, None) => lines.push(line(
            "warning",
            "The server could not tell its address for Collabora yet: the editor will use the panel's address.".into(),
            "Сервер ещё не определил свой адрес для Collabora: редактор возьмёт адрес панели.".into(),
        )),
    }
    // A common slip: the address of Collabora itself instead of the cloud's.
    if settings
        .wopi_base_url
        .as_deref()
        .and_then(web::origin)
        .is_some_and(|base| Some(base) == web::origin(&office_url))
    {
        lines.push(line(
            "warning",
            "“Address of THIS cloud” is the address of Collabora itself. Usually it stays empty; otherwise Collabora answers “Unauthorized WOPI host”.".into(),
            "«Адрес ЭТОГО облака» — это адрес самой Collabora. Обычно настройка пустая; иначе Collabora ответит «Unauthorized WOPI host».".into(),
        ));
    }

    if let Ok(discovery) = &discovery {
        if let Some(theme) = &settings.theme {
            if theme_found(backend, &office_url, discovery, theme) {
                lines.push(line(
                    "ok",
                    format!("Theme “{theme}” is in Collabora"),
                    format!("Тема «{theme}» есть в Collabora"),
                ));
            } else {
                lines.push(line(
                    "warning",
                    format!("Collabora has no theme “{theme}”: the editor looks standard. Its folder goes to /usr/share/coolwsd/browser/dist/{theme}"),
                    format!("В Collabora нет темы «{theme}»: редактор будет стандартным. Её папка — /usr/share/coolwsd/browser/dist/{theme}"),
                ));
            }
        }
        let editable = Discovery::editable(discovery).join(", ");
        if !editable.is_empty() {
            lines.push(line("info", format!("Edits: {editable}"), format!("Редактирует: {editable}")));
        }
    }
    lines
}

/// Whether Collabora serves the theme's `branding.css` next to its editor
/// page (`/browser/<hash>/<theme>/branding.css`), as it does when the folder
/// `browser/dist/<theme>` exists.
fn theme_found(backend: &dyn Backend, office_url: &str, discovery: &Discovery, theme: &str) -> bool {
    let Some(action) = discovery.actions.first() else { return false };
    let page = discovery::editor_url(&action.urlsrc, office_url, "", "");
    let Some(dir) = page.split('?').next().and_then(|page| page.rsplit_once('/')).map(|(dir, _)| dir) else {
        return false;
    };
    backend.fetch(&format!("{dir}/{theme}/branding.css")).is_ok_and(|answer| answer.status == 200)
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
                // Through the panel's address; Collabora calls back the server's inner one.
                "https://cloud.example.com/browser/0c4e5b1/cool.html?WOPISrc=http%3A%2F%2F172.19.0.3%3A8080\
                 %2Fapp%2Frc-office%2Fwopi%2Ffiles%2F{FILE}&closebutton=1&lang=ru"
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
    fn fields_of_the_look() {
        let fake = fake();
        let (_, body) = open_with(&fake, &query());
        assert_eq!(body["fields"], json!({ "ui_defaults": "UIMode=tabbed;SavedUIState=true", "ui_theme": "dark" }));
        let mut fake = fake;
        fake.settings = json!({ "office_url": "http://office.test:9980", "compact_ui": true, "theme": " nextcloud " });
        let (_, body) = open_with(&fake, &query().replace("theme=dark", "theme=light"));
        assert_eq!(
            body["fields"],
            json!({ "ui_defaults": "UIMode=compact;SavedUIState=true", "ui_theme": "light", "theme": "nextcloud" })
        );
        fake.settings = json!({ "office_url": "http://office.test:9980", "theme": "../x" });
        let (_, body) = open_with(&fake, &query());
        assert!(body["fields"].get("theme").is_none());
    }

    #[test]
    fn wopi_base_without_self_url() {
        let fake = Fake { self_url: None, ..fake() };
        let (_, body) = open_with(&fake, &query());
        assert!(body["url"].as_str().unwrap().contains("WOPISrc=https%3A%2F%2Fcloud.example.com%2Fapp%2Frc-office"));
    }

    fn report_of(fake: &Fake) -> Vec<serde_json::Value> {
        serde_json::from_slice(&status(fake).body).unwrap()
    }

    fn has(lines: &[serde_json::Value], level: &str, en: &str) -> bool {
        lines.iter().any(|line| line["level"] == level && line["text"]["en"].as_str().unwrap().contains(en))
    }

    #[test]
    fn status_tells_self_url() {
        let lines = report_of(&fake());
        assert!(has(&lines, "info", "at http://172.19.0.3:8080 (found by the server)"), "{lines:?}");
        assert!(lines.iter().all(|line| line["text"]["ru"].is_string()));
        let lines = report_of(&Fake { self_url: None, ..fake() });
        assert!(has(&lines, "warning", "panel's address"), "{lines:?}");
    }

    #[test]
    fn warns_when_wopi_base_is_collabora() {
        let mut fake = fake();
        fake.settings = json!({ "office_url": "http://office.test:9980/", "wopi_base_url": "http://OFFICE.test:9980" });
        assert!(has(&report_of(&fake), "warning", "Unauthorized WOPI host"));
        fake.settings = json!({ "office_url": "http://office.test:9980/", "wopi_base_url": "http://rustcloud:8080" });
        let lines = report_of(&fake);
        assert!(!has(&lines, "warning", "Unauthorized WOPI host"));
        assert!(has(&lines, "info", "at http://rustcloud:8080 (from the settings)"));
    }

    #[test]
    fn checks_the_theme() {
        let mut fake = fake();
        fake.settings = json!({ "office_url": "http://office.test:9980", "theme": "rustcloud" });
        assert!(has(&report_of(&fake), "warning", "no theme “rustcloud”"));
        let css = "http://office.test:9980/browser/0c4e5b1/rustcloud/branding.css";
        fake.web.borrow_mut().insert(css.into(), Ok(Fetched { status: 200, body: b"a{}".to_vec() }));
        assert!(has(&report_of(&fake), "ok", "Theme “rustcloud”"));
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
        let lines = report_of(&fake);
        assert!(has(&lines, "ok", "Collabora Online Development Edition 26.04.5.2 answers"), "{lines:?}");
        assert!(has(&lines, "info", "docx"));

        let lines = report_of(&Fake::default());
        assert!(has(&lines, "error", "does not answer"), "{lines:?}");

        let unset = Fake { settings: json!({ "office_url": "" }), ..Fake::default() };
        let lines = report_of(&unset);
        assert_eq!(lines.len(), 1);
        assert!(has(&lines, "error", "not set"));
    }
}
