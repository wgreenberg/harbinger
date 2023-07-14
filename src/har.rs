use anyhow::Result;
use base64::{engine::general_purpose::STANDARD, Engine};
use har::{
    v1_2::{Entries, Headers, Log},
    Har as HarExt,
};
use log::warn;
use rocket::http::{uri, Method};
use std::{
    collections::HashMap,
    fs::File,
    path::{Path, PathBuf},
};

use crate::error::HarbingerError;

fn read_v1_2_har(path: &Path) -> Result<Log> {
    let reader = File::open(path)?;
    match serde_json::from_reader::<File, HarExt>(reader)?.log {
        har::Spec::V1_2(log) => Ok(log),
        _ => Err(HarbingerError::UnsupportedHarVersion.into()),
    }
}

pub struct Har {
    pub entries: Vec<Entry>,
    pub page_id: String,
}

impl Har {
    pub fn new(mut har: Log) -> Self {
        let pages = har.pages.as_ref().unwrap();
        if pages.len() > 1 {
            warn!("multiple HAR pages not supported, only using first page");
        }
        let page_id = pages[0].id.clone();
        let entries = har
            .entries
            .drain(..)
            .map(|entry| {
                if entry.pageref.as_ref() != Some(&page_id) {
                    warn!(
                        "entry {}: expected pagref {:?}, got {}",
                        &entry.request.url, &entry.pageref, &page_id
                    );
                }
                Entry::new(entry)
            })
            .collect();
        Har { entries, page_id }
    }

    pub fn entries(&self) -> Result<HashMap<(Method, String), Vec<&Entry>>> {
        let mut map = HashMap::new();
        for entry in &self.entries {
            let method = entry.method()?;
            let uri = entry.uri()?;
            let uri_without_query_or_fragment =
                format!("{}{}", uri.authority().unwrap(), uri.path());
            let matching_entries = map
                .entry((method, uri_without_query_or_fragment))
                .or_insert(Vec::new());
            matching_entries.push(entry);
        }
        Ok(map)
    }

    pub fn read(path: &Path) -> Result<Self> {
        let log = read_v1_2_har(path)?;
        Ok(Har::new(log))
    }

    pub fn primary_url(&self) -> &str {
        &self.entries[0].inner.request.url
    }

    pub fn origin_host(&self) -> Result<String> {
        let uri = self.entries[0].uri()?;
        let host = uri.authority().unwrap().host().to_string();
        Ok(host)
    }
}

#[derive(Clone)]
pub struct Entry {
    inner: Entries,
}

// truncates a string to a given length, less the size of its md5 hash
fn uniquely_truncate(s: &str, limit: usize) -> String {
    let hash = md5::compute(s);
    let substr = s.get(..limit - 32).unwrap_or(s);
    format!("{}_{:x}", substr, hash)
}

impl Entry {
    pub fn new(inner: Entries) -> Entry {
        Entry { inner }
    }

    pub fn get_dump_path(&self, base_path: &Path) -> Result<PathBuf> {
        let url = self
            .inner
            .request
            .url
            .replace("http://", "")
            .replace("https://", "");
        let mut path = base_path.to_path_buf();
        path.push(self.method()?.to_string());
        for part in Path::new(&url).components() {
            if part.as_os_str().len() > 200 {
                let s = part.as_os_str().to_str().unwrap();
                path.push(uniquely_truncate(s, 200))
            } else {
                path.push(part);
            }
        }
        if url.ends_with('/') {
            path.push("__index__");
        }
        Ok(path)
    }

    pub fn uri(&self) -> Result<uri::Reference> {
        let req_uri = self.inner.request.url.as_str();
        let parsed = uri::Uri::parse::<uri::Reference>(req_uri).map_err(|err| {
            dbg!(err);
            HarbingerError::InvalidHarEntryUri {
                uri: req_uri.to_string(),
            }
        })?;
        parsed.reference().cloned().ok_or(
            HarbingerError::InvalidHarEntryUri {
                uri: req_uri.to_string(),
            }
            .into(),
        )
    }

    pub fn method(&self) -> Result<Method> {
        let method_str = self.inner.request.method.as_str();
        method_str.parse::<Method>().map_err(|_| {
            HarbingerError::InvalidHarEntryMethod {
                method: method_str.to_string(),
            }
            .into()
        })
    }

    pub fn hostname(&self) -> Result<String> {
        Ok(self.uri()?.authority().unwrap().host().to_string())
    }

    fn get_header_value<'a>(&self, headers: &'a [Headers], name: &str) -> Option<&'a str> {
        headers
            .iter()
            .filter(|hdr| hdr.name.to_lowercase() == name)
            .map(|hdr| &*hdr.value)
            .next()
    }

    pub fn res_header(&self, name: &str) -> Option<&str> {
        self.get_header_value(&self.inner.response.headers, name)
    }

    pub fn res_headers(&self) -> impl Iterator<Item = (&str, &str)> {
        self.inner
            .response
            .headers
            .iter()
            .map(|header| (header.name.as_str(), header.value.as_str()))
    }

    pub fn status(&self) -> i64 {
        self.inner.response.status
    }

    pub fn res_body(&self) -> Option<Vec<u8>> {
        let body = self.inner.response.content.text.as_ref()?;
        // check if the content is base64 encoded
        if let Ok(decoded) = STANDARD.decode(body) {
            Some(decoded)
        } else {
            Some(body.as_bytes().to_vec())
        }
    }
}
