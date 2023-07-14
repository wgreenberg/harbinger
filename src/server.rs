use anyhow::Result;
use log::{info, warn};
use rocket::config::Config as RocketConfig;
use rocket::http::{uri, ContentType, Status};
use rocket::route::{Handler, Outcome};
use rocket::{get, routes, Response, State};
use rocket::{http::Method, Build, Data, Request, Rocket, Route};
use std::io;
use std::path::PathBuf;

use crate::har::{Entry, Har};

const UNFORWARDED_HEADERS: &[&str] = &[
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

fn get_entry_route_path(entry_uri: &uri::Reference, origin_host: &str) -> Result<String> {
    let hostname = entry_uri.authority().unwrap().host();
    if hostname == origin_host {
        Ok(format!("/{}", entry_uri.path()))
    } else {
        Ok(format!("/{}{}", hostname, entry_uri.path()))
    }
}

pub fn build_server(
    har: &Har,
    port: u16,
    dump_path: Option<&PathBuf>,
    proxy: Option<&reqwest::Url>,
) -> Result<Rocket<Build>> {
    if let Some(path) = dump_path {
        if !path.try_exists().unwrap() {
            panic!("dump path {} doesn't exist", path.display());
        }
    }

    let origin_host = har.origin_host()?;

    let mut entry_routes = Vec::new();
    let mut routed_paths = Vec::new();
    for ((method, path), entries) in har.entries()?.iter() {
        let handler = EntryHandler {
            entries: entries.iter().cloned().cloned().collect(),
            dump_path: dump_path.cloned(),
        };
        let route_path = get_entry_route_path(&entries[0].uri()?, &origin_host)?;
        entry_routes.push(Route::new(*method, &route_path, handler));
        routed_paths.push(path);
    }

    if let Some(proxy_url) = proxy {
        use rocket::http::Method::*;
        for method in &[Get, Put, Post, Delete, Options, Head, Trace, Connect, Patch] {
            let handler = ProxyHandler {
                proxy_url: proxy_url.clone(),
            };
            entry_routes.push(Route::new(*method, "/<any..>", handler));
        }
    }

    let server_config = RocketConfig::figment()
        .merge(("port", port))
        .merge(("log_level", "debug"));

    let shared_config = Config { port, origin_host };

    Ok(rocket::custom(server_config)
        .mount("/", routes![serve_index, serve_app_js, serve_worker_js])
        .mount("/", entry_routes)
        .manage(shared_config))
}

#[derive(Clone)]
struct ProxyHandler {
    proxy_url: reqwest::Url,
}

#[rocket::async_trait]
impl Handler for ProxyHandler {
    async fn handle<'r>(&self, req: &'r Request<'_>, _: Data<'r>) -> Outcome<'r> {
        let client = reqwest::Client::new();
        let method = match req.method() {
            Method::Get => reqwest::Method::GET,
            Method::Put => reqwest::Method::PUT,
            Method::Post => reqwest::Method::POST,
            Method::Delete => reqwest::Method::DELETE,
            Method::Options => reqwest::Method::OPTIONS,
            Method::Head => reqwest::Method::HEAD,
            Method::Trace => reqwest::Method::TRACE,
            Method::Connect => reqwest::Method::CONNECT,
            Method::Patch => reqwest::Method::PATCH,
        };
        let mut proxy_url = self.proxy_url.clone();
        proxy_url.set_path(req.uri().path().as_str());
        if let Some(query) = req.uri().query().as_ref() {
            proxy_url.set_query(Some(query.as_str()));
        }
        let proxy_req = client.request(method, proxy_url).build().unwrap();
        let proxy_res = client.execute(proxy_req).await.unwrap();
        let mut res = Response::new();
        let status = Status::from_code(proxy_res.status().as_u16()).unwrap();
        res.set_status(status);
        for (name, value) in proxy_res.headers() {
            let name_clone = name.to_string();
            let value_clone = value.to_str().unwrap().to_string();
            res.adjoin_raw_header(name_clone, value_clone);
        }
        if let Ok(bytes) = proxy_res.bytes().await {
            res.set_sized_body(bytes.len(), io::Cursor::new(bytes));
        }
        Outcome::Success(res)
    }
}

#[derive(Clone)]
struct EntryHandler {
    entries: Vec<Entry>,
    dump_path: Option<PathBuf>,
}

impl EntryHandler {
    fn get_body(&self, entry: &Entry) -> Result<Vec<u8>> {
        if let Some(base_path) = &self.dump_path {
            let override_path = entry.get_dump_path(base_path)?;
            if override_path.exists() {
                info!(
                    "{} {}: loading body from file {}",
                    entry.method()?,
                    entry.uri()?,
                    override_path.display()
                );
                return std::fs::read(override_path).map_err(|err| err.into());
            }
        }
        info!(
            "{} {}: loading body from HAR",
            entry.method()?,
            entry.uri()?
        );
        Ok(entry.res_body().unwrap_or(vec![]))
    }
}

#[rocket::async_trait]
impl Handler for EntryHandler {
    // handler for a group of entries that share the same path
    async fn handle<'r>(&self, req: &'r Request<'_>, data: Data<'r>) -> Outcome<'r> {
        for entry in &self.entries {
            if req.uri().query() == entry.uri().unwrap().query() {
                let mut res = Response::new();
                for (name, value) in entry.res_headers() {
                    let normalized_name = name.to_ascii_lowercase();
                    if UNFORWARDED_HEADERS.contains(&normalized_name.as_str()) {
                        continue;
                    }

                    // handle Location headers for redirects
                    if normalized_name == "location" {
                        let hostname = entry.hostname().unwrap();
                        let new_location = if value.starts_with('/') {
                            format!("/{}{}", hostname, value)
                        } else {
                            format!("/{}/{}", hostname, value)
                        };
                        res.set_raw_header(name.to_string(), new_location);
                    } else {
                        res.set_raw_header(name.to_string(), value.to_string());
                    }
                }
                let csp_components = [
                    "base-uri 'self'",
                    "default-src * 'unsafe-inline' 'unsafe-eval'",
                    "worker-src 'self'",
                ];
                res.set_raw_header("content-security-policy", csp_components.join("; "));
                match self.get_body(entry) {
                    Ok(body) => res.set_sized_body(None, io::Cursor::new(body)),
                    Err(err) => {
                        warn!("entry failed to handle request: {:?}", err);
                        return Outcome::Failure(Status::InternalServerError);
                    }
                }
                res.set_status(rocket::http::Status::new(entry.status() as u16));
                return Outcome::Success(res);
            }
        }
        Outcome::Forward(data)
    }
}
