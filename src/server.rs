use anyhow::Result;
use rocket::{Build, Rocket};
use std::path::PathBuf;

use crate::har::Har;

pub fn build_server(har: &Har, dump_path: &Option<PathBuf>) -> Result<Rocket<Build>> {
    if let Some(path) = dump_path {
        if !path.try_exists().unwrap() {
            panic!("dump path {} doesn't exist", path.display());
        }
    }

    Ok(rocket::build())
}
