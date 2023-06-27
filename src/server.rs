use anyhow::Result;
use log::{error, info, warn};
use rocket::config::Config as RocketConfig;
use rocket::http::{uri, ContentType, Status};
use rocket::route::{Handler, Outcome};
use rocket::{get, routes, Response, State};
use rocket::{http::Method, Build, Data, Request, Rocket, Route};
use std::io;
use std::path::PathBuf;

use crate::{
    error::HarbingerError,
    har::{Entry, Har},
};

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
    origin_host: String,
}

#[get("/harbinger")]
fn serve_index() -> (ContentType, &'static str) {
    let content = include_str!("../static/index.html");
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
        .replace("HARBINGER_TMPL_PORT", &config.port.to_string())
        .replace("HARBINGER_TMPL_ORIGIN_HOST", &config.origin_host);
    (ContentType::JavaScript, content)
}

fn get_entry_route_path(entry_uri: &uri::Absolute, origin_host: &str) -> Result<String> {
    let hostname = entry_uri.authority().unwrap().host();
    if hostname == origin_host {
        Ok(format!("/{}", entry_uri.path()))
    } else {
        Ok(format!("/{}{}", hostname, entry_uri.path()))
    }
}

pub fn build_server(har: &Har, port: u16, dump_path: &Option<PathBuf>) -> Result<Rocket<Build>> {
    if let Some(path) = dump_path {
        if !path.try_exists().unwrap() {
            panic!("dump path {} doesn't exist", path.display());
        }
    }

    let origin_host = har.origin_host()?;

    let mut entry_routes = Vec::new();
    let mut routed_paths = Vec::new();
    for entry in har.entries.iter().cloned() {
        let entry_uri = entry.uri()?;
        let path = get_entry_route_path(&entry_uri, &origin_host)?;
        if routed_paths.contains(&path) {
            warn!("found duplicate entry for path {}, skipping", &path);
            continue;
        }
        let method: Method =
            entry
                .method()
                .parse()
                .map_err(|_| HarbingerError::InvalidHarEntryMethod {
                    method: entry.method().to_string(),
                })?;
        let handler = EntryHandler {
            entry,
            dump_path: dump_path.clone(),
        };
        entry_routes.push(Route::new(method, &path, handler));
        routed_paths.push(path);
    }

    let server_config = RocketConfig::figment().merge(("port", port));

    let shared_config = Config { port, origin_host };

    Ok(rocket::custom(server_config)
        .mount("/", routes![serve_index, serve_app_js, serve_worker_js])
        .mount("/", entry_routes)
        .manage(shared_config))
}

#[derive(Clone)]
struct EntryHandler {
    entry: Entry,
    dump_path: Option<PathBuf>,
}

impl EntryHandler {
    fn get_body(&self) -> Result<Vec<u8>> {
        if let Some(base_path) = &self.dump_path {
            let override_path = self.entry.get_dump_path(base_path);
            if override_path.exists() {
                info!(
                    "{} {}: loading body from file {}",
                    self.entry.method(),
                    self.entry.uri()?,
                    override_path.display()
                );
                return std::fs::read(override_path).map_err(|err| err.into());
            }
        }
        info!(
            "{} {}: loading body from HAR",
            self.entry.method(),
            self.entry.uri()?
        );
        Ok(self.entry.res_body().unwrap_or(vec![]).to_owned())
    }
}

#[rocket::async_trait]
impl Handler for EntryHandler {
    async fn handle<'r>(&self, _req: &'r Request<'_>, _data: Data<'r>) -> Outcome<'r> {
        let mut res = Response::new();
        for (name, value) in self.entry.res_headers() {
            let normalized_name = name.to_ascii_lowercase();
            if UNFORWARDED_HEADERS.contains(&normalized_name.as_str()) {
                continue;
            }

            // handle Location headers for redirects
            if normalized_name == "location" {
                let hostname = self.entry.hostname().unwrap();
                let new_location;
                if value.starts_with('/') {
                    new_location = format!("/{}{}", hostname, value);
                } else {
                    new_location = format!("/{}/{}", hostname, value);
                }
                res.set_raw_header(name.to_string(), new_location);
            } else {
                res.set_raw_header(name.to_string(), value.to_string());
            }
        }
        let csp_components = [
            "base-uri 'self'",
            "default-src * 'unsafe-inline'",
            //"script-src 'self' 'unsafe-inline'",
            //"style-src 'self' 'unsafe-inline'",
            //"connect-src 'self'",
            //"font-src 'self'",
            //"object-src 'self'",
            //"child-src 'self'",
            //"media-src 'self'",
            "frame-src 'self'",
            //"img-src 'self'",
            "worker-src 'self'",
            "manifest-src 'self'",
            //"form-action 'self'",
        ];
        res.set_raw_header("content-security-policy", csp_components.join("; "));
        match self.get_body() {
            Ok(body) => {
                res.set_sized_body(None, io::Cursor::new(body));
            }
            Err(err) => {
                error!("error getting body for entry {}", err);
                return Outcome::Failure(Status::InternalServerError);
            }
        }
        res.set_status(rocket::http::Status::new(self.entry.status() as u16));
        Outcome::Success(res)
    }
}
