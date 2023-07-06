use anyhow::Result;
use base64::{engine::general_purpose::STANDARD, Engine};
use har::{
    v1_2::{Entries, Headers, Log},
    Har as HarExt,
};
use log::warn;
use rocket::http::uri;
use std::{
    fs::File,
    path::{Path, PathBuf},
};

use crate::error::HarbingerError;

fn read_v1_2_har(path: &PathBuf) -> Result<Log> {
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

    pub fn read(path: &PathBuf) -> Result<Self> {
        let log = read_v1_2_har(path)?;
        Ok(Har::new(log))
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

impl Entry {
    pub fn new(inner: Entries) -> Entry {
        Entry { inner }
    }

    pub fn get_dump_path(&self, base_path: &Path) -> PathBuf {
        let url = self
            .inner
            .request
            .url
            .replace("http://", "")
            .replace("https://", "");
        let mut path = base_path.to_path_buf();
        for part in Path::new(&url).components() {
            path.push(part);
        }
        if url.ends_with('/') {
            path.push("__index__");
        }
        path
    }

    pub fn uri(&self) -> Result<uri::Absolute> {
        let req_uri = self.inner.request.url.as_str();
        let parsed = uri::Uri::parse::<uri::Absolute>(req_uri)
            .map_err(|_| HarbingerError::InvalidHarEntryUri)?;
        parsed
            .absolute()
            .cloned()
            .ok_or(HarbingerError::InvalidHarEntryUri.into())
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

    pub fn method(&self) -> &str {
        self.inner.request.method.as_str()
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
