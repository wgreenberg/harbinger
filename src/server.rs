use anyhow::Result;
use log::{info, error, debug};
use rocket::{Response, get, routes, State};
use rocket::{Build, Rocket, Route, http::Method, Request, Data};
use rocket::config::Config as RocketConfig;
use rocket::http::{Status, ContentType};
use rocket::route::{Handler, Outcome};
use std::io;
use std::path::PathBuf;

use crate::{har::{Har, Entry}, error::HarbingerError};

const UNFORWARDED_HEADERS: &'static [&'static str] = &[
    // Security headers we want to override
    "x-frame-options",
    "x-content-type-options",
    "x-xss-protection",
    "access-control-allow-origin",
    "access-control-allow-credentials",

    // Content-specific headers that may be overridden depending on user
    // modifications
    "content-encoding",
    "transfer-encoding",
    "content-length",
];

struct Config {
    port: u16,
    start_uri: String,
}

#[get("/")]
fn serve_index(config: &State<Config>) -> (ContentType, String) {
    let content = include_str!("../static/index.html")
            .replace("HARBINGER_TMPL_START_URI", &config.start_uri);
    (ContentType::HTML, content)
}

#[get("/harbinger_app.js")]
fn serve_app_js() -> (ContentType, &'static str) {
    let content = include_str!("../static/harbinger_app.js");
    (ContentType::JavaScript, content)
}

#[get("/harbinger_worker.js")]
fn serve_worker_js(config: &State<Config>) -> (ContentType, String) {
    let content = include_str!("../static/harbinger_worker.js")
        .replace("HARBINGER_TMPL_PORT", &config.port.to_string());
    (ContentType::JavaScript, content)
}

fn get_entry_route_path(entry: &Entry) -> Result<String> {
    let uri = entry.uri()?;
    let hostname = uri.authority()
        .unwrap()
        .host();
    Ok(format!("/{}{}", hostname, uri.path()))
}

pub fn build_server(har: &Har, port: u16, dump_path: &Option<PathBuf>) -> Result<Rocket<Build>> {
    if let Some(path) = dump_path {
        if !path.try_exists().unwrap() {
            panic!("dump path {} doesn't exist", path.display());
        }
    }

    let origin_host = har.origin_host()?;
    let shared_config = Config {
        port,
        start_uri: format!("/srv/{}", origin_host),
    };

    let mut routes = Vec::new();
    let mut routed_paths = Vec::new();
    for entry in har.entries.iter().cloned() {
        let path = get_entry_route_path(&entry)?;
        dbg!(&path);
        if routed_paths.contains(&path) {
            continue
        }
        let method: Method = entry.method().parse()
            .map_err(|_| HarbingerError::InvalidHarEntryMethod {
                method: entry.method().to_string(),
            })?;
        let handler = EntryHandler { entry, dump_path: dump_path.clone() };
        routes.push(Route::new(method, &path, handler));
        routed_paths.push(path);
    }

    let server_config = RocketConfig::figment()
        .merge(("port", port));

    Ok(rocket::custom(server_config)
        .mount("/", routes![serve_index, serve_app_js, serve_worker_js])
        .mount("/srv", routes)
        .manage(shared_config))
}

#[derive(Clone)]
struct EntryHandler {
    entry: Entry,
    dump_path: Option<PathBuf>,
}

impl EntryHandler {
    fn get_body(&self) -> Result<String> {
        if let Some(base_path) = &self.dump_path {
            let override_path = self.entry.get_dump_path(base_path);
            if override_path.exists() {
                info!("{} {}: loading body from file {}", self.entry.method(), self.entry.uri()?, override_path.display());
                return std::fs::read_to_string(override_path)
                    .map_err(|err| err.into());
            }
        }
        info!("{} {}: loading body from HAR", self.entry.method(), self.entry.uri()?);
        Ok(self.entry.res_body()
            .unwrap_or("").to_owned())
    }
}

#[rocket::async_trait]
impl Handler for EntryHandler {
    async fn handle<'r>(&self, req: &'r Request<'_>, data: Data<'r>) -> Outcome<'r> {
        let mut res = Response::new();
        for (name, value) in self.entry.res_headers() {
            if UNFORWARDED_HEADERS.contains(&name.to_ascii_lowercase().as_str()) {
                continue;
            }
            res.set_raw_header(name.to_string(), value.to_string());
        }
        res.set_raw_header("content-security-policy", "base-uri 'self'");
        match self.get_body() {
            Ok(body) => {
                res.set_sized_body(None, io::Cursor::new(body));
            },
            Err(err) => {
                error!("error getting body for entry {}", err);
                return Outcome::Failure(Status::InternalServerError);
            },
        }
        res.set_status(rocket::http::Status::new(self.entry.status() as u16));
        Outcome::Success(res)
    }
}
