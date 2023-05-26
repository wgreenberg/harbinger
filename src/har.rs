use anyhow::Result;
use har::{
    from_path,
    v1_2::{Entries, Headers, Log},
};
use log::warn;
use rocket::{http::uri::Uri, Route};
use std::{
    convert::TryFrom,
    path::{Path, PathBuf},
};

use crate::error::HarbingerError;

fn read_v1_2_har(path: &PathBuf) -> Result<Log> {
    match from_path(path)?.log {
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
            .filter(|entry| {
                entry
                    .pageref
                    .as_ref()
                    .map(|id| id == &page_id)
                    .unwrap_or(false)
            })
            .map(|entry| Entry::new(entry))
            .collect();
        Har { entries, page_id }
    }

    pub fn read(path: &PathBuf) -> Result<Self> {
        let log = read_v1_2_har(path)?;
        Ok(Har::new(log))
    }
}

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

    pub fn uri(&self) -> Result<rocket::http::uri::Uri> {
        Uri::parse_any(self.inner.request.url.as_str())
            .map_err(|_| HarbingerError::InvalidHarEntryUri.into())
    }

    pub fn is_origin_request(&self, origin_host: &str) -> Result<bool> {
        Ok(match self.uri()? {
            Uri::Origin(_) => true,
            Uri::Absolute(uri) => {
                if let Some(authority) = uri.authority() {
                    authority.host() == origin_host
                } else {
                    true
                }
            }
            _ => false,
        })
    }

    fn get_header_value<'a>(&self, headers: &'a [Headers], name: &str) -> Option<&'a str> {
        headers
            .iter()
            .filter(|hdr| hdr.name.to_lowercase() == name)
            .map(|hdr| &*hdr.value)
            .next()
    }

    pub fn req_header(&self, name: &str) -> Option<&str> {
        self.get_header_value(&self.inner.request.headers, name)
    }

    pub fn res_header(&self, name: &str) -> Option<&str> {
        self.get_header_value(&self.inner.response.headers, name)
    }

    pub fn method(&self) -> &str {
        return self.inner.request.method.as_str();
    }

    pub fn status(&self) -> i64 {
        return self.inner.response.status;
    }

    pub fn res_body(&self) -> Option<&str> {
        self.inner.response.content.text.as_deref()
    }
}

impl TryFrom<&Entry> for Route {
    type Error = HarbingerError;

    fn try_from(value: &Entry) -> std::result::Result<Self, Self::Error> {
        todo!()
    }
}
