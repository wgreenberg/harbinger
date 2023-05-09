use std::path::{PathBuf, Path};
use std::fs::{create_dir, create_dir_all, OpenOptions};
use std::io::Write;
use clap::{Parser, Subcommand};
use thiserror::Error;
use har::{from_path, v1_2::{Log, Entries, Headers}};
use rocket::{
    Rocket,
    Build,
};

#[derive(Error, Debug)]
enum Error {
    #[error("io error")]
    IoError(#[from] std::io::Error),
    #[error("unsupported HAR version")]
    UnsupportedHarVersion,
    #[error("HAR error")]
    HarError(#[from] har::Error),
    #[error("dump path exists! cowardly bailing")]
    DumpPathExists,
}

#[derive(Parser, Debug)]
struct Args {
    #[command(subcommand)]
    command: Command,
}

impl Args {
    fn get_path(&self) -> &PathBuf {
        match &self.command {
            Command::Serve { har_path, .. } => har_path,
            Command::Dump { har_path, .. } => har_path,
        }
    }
}

#[derive(Subcommand, Debug)]
enum Command {
    Serve {
        har_path: PathBuf,

        #[arg(long, short)]
        dump_path: Option<PathBuf>,
    },
    Dump {
        har_path: PathBuf,

        #[arg(long, short)]
        output_path: PathBuf,

        #[arg(long)]
        unminify: bool,
    },
}

fn build_server(har: &Log, dump_path: &Option<PathBuf>) -> Rocket<Build> {
    let server = rocket::build();
    server
}

fn dump(har: &Log, output_path: &PathBuf, unminify: bool) -> Result<(), Error> {
    if !output_path.try_exists()? {
        println!("creating {}...", output_path.display());
        create_dir(output_path)?;
    } else {
        return Err(Error::DumpPathExists);
    }

    let page_id = har.pages.as_ref().map(|pages| {
        if pages.len() > 1 {
            eprintln!("multiple HAR pages not supported, only using first page");
        }
        pages[0].id.clone()
    });

    let entries = har.entries.iter()
        .filter(|entry| entry.pageref == page_id);

    for entry in entries {
        if entry.request.method != "GET" {
            continue;
        }

        let _ext = match entry_content_type(entry) {
            Some("application/javascript") => ".js",
            Some("application/json") => ".json",
            Some("text/html") => ".html",
            _ => continue,
        };
        let path = entry_to_dump_path(output_path, &entry);
        match path.parent() {
            Some(parent_path) => create_dir_all(parent_path)?,
            None => {},
        }
        println!("creating {}", path.display());
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .open(path)?;
        let body = entry.response.content.text.as_ref().unwrap();
        file.write_all(body.as_bytes())?;
    }

    Ok(())
}

fn entry_to_dump_path(base_path: &PathBuf, entry: &Entries) -> PathBuf {
    let url = entry.request.url
        .replace("http://", "")
        .replace("https://", "");
    let mut path = base_path.clone();
    for part in Path::new(&url).components() {
        path.push(part);
    }
    if url.ends_with('/') {
        path.push("__index__");
    }
    path
}

fn get_header_value<'a>(headers: &'a Vec<Headers>, name: &str) -> Option<&'a str> {
    headers.iter()
        .filter(|hdr| hdr.name.to_lowercase() == name)
        .map(|hdr| &*hdr.value)
        .next()
}

fn entry_content_type<'a>(entry: &'a Entries) -> Option<&'a str> {
    get_header_value(&entry.response.headers, "content-type")
}

fn read_v1_2_har(path: &PathBuf) -> Result<Log, Error> {
    match from_path(path)?.log {
        har::Spec::V1_2(log) => Ok(log),
        _ => Err(Error::UnsupportedHarVersion),
    }
}

#[rocket::main]
async fn main () {
    let args = Args::parse();
    let har = read_v1_2_har(args.get_path()).unwrap();
    match &args.command {
        Command::Serve { dump_path, .. } => {
            let _ = build_server(&har, &dump_path)
                .launch()
                .await;
        },
        Command::Dump { output_path, unminify, .. } => {
            dump(&har, &output_path, *unminify)
                .expect("failed to dump HAR");
        },
    }
}
