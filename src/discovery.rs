//! WOPI discovery of Collabora Online: which editor address (`urlsrc`) opens
//! which file. Fetched from `{office_url}/hosting/discovery`, kept in the
//! plugin's storage (it is the same for all users) for an hour.

use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;
use serde::{Deserialize, Serialize};

use crate::backend::Backend;
use crate::web;

/// Where the parsed discovery is kept in the plugin's storage.
pub const CACHE: &str = "discovery.json";
/// How long a discovery is used before it is fetched again.
pub const FRESH_MS: i64 = 3600 * 1000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Discovery {
    /// The `office_url` it was fetched from.
    pub office_url: String,
    pub fetched_ms: i64,
    pub actions: Vec<Action>,
}

/// An `<action>` of an `<app>`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Action {
    /// The `<app>`'s name: a MIME type, or `writer`, `calc`…
    pub app: String,
    /// The file extension, lower case; empty when the app is named by MIME type.
    pub ext: String,
    /// `edit`, `view`, `view_comment`…
    pub name: String,
    pub urlsrc: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// No answer, or not 200.
    Unreachable(String),
    /// An answer, but no discovery in it.
    Invalid(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Unreachable(why) => write!(f, "Collabora Online does not answer: {why}"),
            Error::Invalid(why) => write!(f, "Collabora Online answered without a discovery: {why}"),
        }
    }
}

/// The discovery of `office_url`: from the cache while it is fresh (unless
/// `refresh`), else fetched. If fetching fails, an older one of the same
/// address still serves.
pub fn get(backend: &dyn Backend, office_url: &str, refresh: bool) -> Result<Discovery, Error> {
    let cached = backend
        .storage_read(CACHE)
        .and_then(|bytes| serde_json::from_slice::<Discovery>(&bytes).ok())
        .filter(|cached| cached.office_url == office_url);
    let now = backend.now_ms();
    if let Some(cached) = &cached {
        if !refresh && (0..FRESH_MS).contains(&(now - cached.fetched_ms)) {
            return Ok(cached.clone());
        }
    }
    let fetched = fetch(backend, office_url).and_then(|xml| {
        let actions = parse(&xml)?;
        Ok(Discovery { office_url: office_url.to_owned(), fetched_ms: now, actions })
    });
    match (fetched, cached) {
        (Ok(discovery), _) => {
            if let Err(why) = backend.storage_write(CACHE, &serde_json::to_vec(&discovery).unwrap_or_default()) {
                backend.warn(&format!("discovery not cached: {why}"));
            }
            Ok(discovery)
        }
        (Err(why), Some(cached)) => {
            backend.warn(&format!("{why}; the discovery of an hour ago serves"));
            Ok(cached)
        }
        (Err(why), None) => Err(why),
    }
}

fn fetch(backend: &dyn Backend, office_url: &str) -> Result<String, Error> {
    let answer = backend.fetch(&format!("{office_url}/hosting/discovery")).map_err(Error::Unreachable)?;
    if answer.status != 200 {
        return Err(Error::Unreachable(format!("HTTP {}", answer.status)));
    }
    String::from_utf8(answer.body).map_err(|_| Error::Invalid("not UTF-8".into()))
}

/// The `<action>`s of a discovery XML.
pub fn parse(xml: &str) -> Result<Vec<Action>, Error> {
    let mut reader = Reader::from_str(xml);
    let mut app = String::new();
    let mut actions = Vec::new();
    let mut root = false;
    loop {
        match reader.read_event() {
            Ok(Event::Start(element)) | Ok(Event::Empty(element)) => match element.local_name().as_ref() {
                b"wopi-discovery" => root = true,
                b"app" => app = attribute(&element, "name").unwrap_or_default(),
                b"action" => {
                    let (Some(name), Some(urlsrc)) = (attribute(&element, "name"), attribute(&element, "urlsrc"))
                    else {
                        continue;
                    };
                    let ext = attribute(&element, "ext").unwrap_or_default().to_ascii_lowercase();
                    actions.push(Action { app: app.clone(), ext, name, urlsrc });
                }
                _ => {}
            },
            Ok(Event::End(element)) if element.local_name().as_ref() == b"app" => app.clear(),
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(err) => return Err(Error::Invalid(format!("XML: {err}"))),
        }
    }
    if !root {
        return Err(Error::Invalid("no <wopi-discovery>".into()));
    }
    if actions.is_empty() {
        return Err(Error::Invalid("no <action>".into()));
    }
    Ok(actions)
}

fn attribute(element: &BytesStart<'_>, name: &str) -> Option<String> {
    let attribute = element.try_get_attribute(name).ok()??;
    attribute.unescape_value().ok().map(|value| value.into_owned())
}

impl Discovery {
    /// The action that opens a file `name` of type `mime`: to edit if
    /// `write` and there is one, else to view (an editor opened read-only
    /// still serves to view). By extension first, then by MIME type.
    pub fn find(&self, name: &str, mime: Option<&str>, write: bool) -> Option<&Action> {
        let ext = name.rsplit_once('.').map(|(_, ext)| ext.to_ascii_lowercase()).unwrap_or_default();
        let wanted: &[&str] = if write { &["edit", "view"] } else { &["view", "edit"] };
        let by_ext = |action: &&Action| !ext.is_empty() && action.ext == ext;
        let by_mime = |action: &&Action| mime.is_some_and(|mime| action.app.eq_ignore_ascii_case(mime));
        for matches in [&by_ext as &dyn Fn(&&Action) -> bool, &by_mime] {
            for wanted in wanted {
                if let Some(action) = self.actions.iter().filter(matches).find(|action| action.name == *wanted) {
                    return Some(action);
                }
            }
        }
        None
    }

    /// Extensions Collabora edits.
    pub fn editable(&self) -> Vec<String> {
        let mut exts: Vec<String> = self
            .actions
            .iter()
            .filter(|action| action.name == "edit" && !action.ext.is_empty())
            .map(|action| action.ext.clone())
            .collect();
        exts.sort();
        exts.dedup();
        exts
    }
}

/// The editor's address for a document: `urlsrc` with its placeholders
/// (`<ui=UI_LLCC&>`) dropped, on the origin of `office_url` (browsers reach
/// Collabora there, and the panel lets frames only there), with `WOPISrc`
/// and the user's language and theme.
pub fn editor_url(urlsrc: &str, office_url: &str, wopi_src: &str, lang: &str, dark: bool) -> String {
    let mut url = String::with_capacity(urlsrc.len());
    let mut placeholder = false;
    for c in urlsrc.chars() {
        match c {
            '<' => placeholder = true,
            '>' => placeholder = false,
            c if !placeholder => url.push(c),
            _ => {}
        }
    }
    if let (Some(from), Some(to)) = (web::origin(&url), web::origin(office_url)) {
        url = format!("{to}{}", &url[from.len()..]);
    }
    if !url.contains('?') {
        url.push('?');
    } else if !url.ends_with('?') && !url.ends_with('&') {
        url.push('&');
    }
    url.push_str(&format!("WOPISrc={}", web::encode(wopi_src)));
    if !lang.is_empty() {
        url.push_str(&format!("&lang={}", web::encode(lang)));
    }
    if dark {
        url.push_str("&ui_theme=dark");
    }
    url
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::fake::Fake;
    use crate::backend::Fetched;

    /// Shaped as Collabora CODE answers.
    pub const XML: &str = include_str!("../tests/discovery.xml");

    fn discovery() -> Discovery {
        Discovery { office_url: "http://office.test:9980".into(), fetched_ms: 0, actions: parse(XML).unwrap() }
    }

    #[test]
    fn parses() {
        let actions = parse(XML).unwrap();
        assert!(actions.iter().any(|a| a.ext == "docx" && a.name == "edit"));
        assert!(actions.iter().any(|a| a.ext == "pdf" && a.name == "view"));
        assert!(actions.iter().any(|a| a.app == "application/vnd.oasis.opendocument.text" && a.ext.is_empty()));
        assert!(actions
            .iter()
            .filter(|a| a.name == "edit")
            .all(|a| a.urlsrc.starts_with("http://collabora.internal:9980/browser/")));
        assert!(matches!(parse("<html/>"), Err(Error::Invalid(_))));
        assert!(matches!(parse("<wopi-discovery></wopi-discovery>"), Err(Error::Invalid(_))));
        assert!(matches!(parse("<wopi-discovery><app"), Err(Error::Invalid(_))));
    }

    #[test]
    fn finds_actions() {
        let discovery = discovery();
        assert_eq!(discovery.find("Report.DOCX", None, true).unwrap().name, "edit");
        assert_eq!(discovery.find("Report.pdf", None, true).unwrap().name, "view");
        assert_eq!(
            discovery.find("Notes", Some("application/vnd.oasis.opendocument.text"), true).unwrap().name,
            "edit"
        );
        assert!(discovery.find("photo.jpg", Some("image/jpeg"), true).is_none());
        assert!(discovery.find("noext", None, false).is_none());
        let editable = discovery.editable();
        assert!(editable.contains(&"odt".to_owned()) && editable.contains(&"xlsx".to_owned()));
        assert!(!editable.contains(&"pdf".to_owned()));
    }

    #[test]
    fn builds_the_editor_url() {
        let url = editor_url(
            "http://collabora.internal:9980/browser/abc/cool.html?<ui=UI_LLCC&>",
            "https://office.example.com",
            "https://cloud.example.com/app/rc-office/wopi/files/f1",
            "ru",
            true,
        );
        assert_eq!(
            url,
            "https://office.example.com/browser/abc/cool.html?\
             WOPISrc=https%3A%2F%2Fcloud.example.com%2Fapp%2Frc-office%2Fwopi%2Ffiles%2Ff1&lang=ru&ui_theme=dark"
        );
        let plain = editor_url("http://o:1/x?a=1", "http://o:1", "w", "", false);
        assert_eq!(plain, "http://o:1/x?a=1&WOPISrc=w");
    }

    #[test]
    fn caches() {
        let fake = Fake::default();
        let url = "http://office.test:9980/hosting/discovery";
        fake.web.borrow_mut().insert(url.into(), Ok(Fetched { status: 200, body: XML.into() }));
        let first = get(&fake, "http://office.test:9980", false).unwrap();
        assert_eq!(fake.fetches.borrow().len(), 1);
        assert!(fake.storage.borrow().contains_key(CACHE));

        // Fresh: no fetch.
        assert_eq!(get(&fake, "http://office.test:9980", false).unwrap(), first);
        assert_eq!(fake.fetches.borrow().len(), 1);
        // Asked to refresh, or another address: fetched.
        get(&fake, "http://office.test:9980", true).unwrap();
        assert_eq!(fake.fetches.borrow().len(), 2);
        assert!(get(&fake, "http://other:9980", false).is_err());
        assert_eq!(fake.fetches.borrow().len(), 3);

        // Old, and Collabora is down: the old one serves.
        let mut later = Fake { now_ms: fake.now_ms + FRESH_MS + 1, ..Fake::default() };
        later.storage = fake.storage.clone();
        assert_eq!(get(&later, "http://office.test:9980", false).unwrap().actions, first.actions);
        assert_eq!(later.fetches.borrow().len(), 1);
    }

    #[test]
    fn reports_failures() {
        let fake = Fake::default();
        assert!(matches!(get(&fake, "http://office.test:9980", false), Err(Error::Unreachable(_))));
        let url = "http://office.test:9980/hosting/discovery";
        fake.web.borrow_mut().insert(url.into(), Ok(Fetched { status: 502, body: Vec::new() }));
        assert!(matches!(get(&fake, "http://office.test:9980", false), Err(Error::Unreachable(_))));
        fake.web.borrow_mut().insert(url.into(), Ok(Fetched { status: 200, body: b"<html></html>".to_vec() }));
        assert!(matches!(get(&fake, "http://office.test:9980", false), Err(Error::Invalid(_))));
    }
}
