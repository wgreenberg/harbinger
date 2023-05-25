mod js;

use std::path::{PathBuf, Path};
use std::fs::{
    create_dir,
    create_dir_all,
    OpenOptions,
};
use std::io::Write;
use clap::{Parser, Subcommand};
use anyhow::Result;
use deno_ast::ParsedSource;
use thiserror::Error;
use har::{
    from_path,
    v1_2::{Log, Entries, Headers},
};
use rocket::{
    Rocket,
    Build,
};
use dprint_plugin_typescript::*;
use dprint_plugin_typescript::configuration::*;
use indicatif::{ProgressBar, ProgressStyle};
use log::{info, warn};

use crate::js::{parse_swc_ast, unpack_webpack_chunk_list};

#[derive(Error, Debug)]
enum HarbingerError {
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

fn dump(har: &Log, output_path: &PathBuf, unminify: bool) -> Result<()> {
    if output_path.try_exists()? {
        return Err(HarbingerError::DumpPathExists.into());
    }
    create_dir(output_path)?;

    let page_id = har.pages.as_ref().map(|pages| {
        if pages.len() > 1 {
            warn!("multiple HAR pages not supported, only using first page");
        }
        pages[0].id.clone()
    });

    let entries: Vec<&Entries> = har.entries.iter()
        .filter(|entry| entry.pageref == page_id)
        .collect();

    let pb_style = ProgressStyle::with_template("{prefix:.bold.dim} {spinner} {bar} {msg}")
        .unwrap();
    let pb = ProgressBar::new(entries.len() as u64);
    pb.set_style(pb_style);

    let unminification_config = ConfigurationBuilder::new()
        .line_width(80)
        .arrow_function_use_parentheses(UseParentheses::Force)
        .prefer_single_line(false)
        .quote_style(QuoteStyle::PreferSingle)
        .next_control_flow_position(NextControlFlowPosition::SameLine)
        .build();

    for (i, entry) in entries.iter().enumerate() {
        pb.set_prefix(format!("[{}/{}]", i, entries.len()));
        pb.set_message(format!("processing {}", entry.request.url));
        if entry.request.method != "GET" {
            pb.println(format!("skipping {} {}", entry.request.method, entry.request.url));
            continue;
        }

        let path = entry_to_dump_path(output_path, &entry);
        match path.parent() {
            Some(parent_path) => create_dir_all(parent_path)?,
            None => {},
        }

        pb.println(format!("processing {}", entry.request.url));
        let body = entry.response.content.text
            .as_ref()
            .unwrap();
        if unminify && entry_content_type(&entry) == Some("application/javascript") {
            pb.println(" * parsing...");
            let parsed_source = parse_swc_ast(&path, &body)?;
            if let Some(chunks) = unpack_webpack_chunk_list(&parsed_source) {
                let mut unpack_path = path.with_extension("");
                let file_name = unpack_path.file_name().unwrap().to_str().unwrap();
                unpack_path.set_file_name(format!("{}_unbundled", file_name));
                pb.println(format!(" * detected {} webpack chunks, unpacking to {}...", chunks.len(), unpack_path.display()));
                create_dir(&unpack_path)?;
                for (id, source) in chunks {
                    let mut chunk_path = unpack_path.join(id);
                    chunk_path.set_extension("js");
                    pb.println(format!("  * unpacking {}...", chunk_path.display()));
                    let parsed_chunk = parse_swc_ast(&chunk_path, &source)?;
                    write_unminified(&chunk_path, &parsed_chunk, &unminification_config)?;
                }
            }
            pb.println(" * unminifying...");
            write_unminified(&path, &parsed_source, &unminification_config)?;
        } else {
            pb.println(" * writing normally...");
            let mut file = OpenOptions::new()
                .write(true)
                .create(true)
                .open(&path)?;
            file.write_all(body.as_bytes())?;
        }
        pb.inc(1);
    }
    pb.inc(1);
    pb.finish_with_message("finished!");

    Ok(())
}

#[derive(Error, Debug)]
enum UnminificationError {
    #[error("no unminified body returned")]
    NoUnminifiedBody,
}

fn write_unminified(path: &PathBuf, source: &ParsedSource, config: &Configuration) -> Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .open(&path)?;
    match format_parsed_source(&source, &config) {
        Ok(Some(unminified_body)) => {
            file.write_all(&unminified_body.as_bytes())?;
            Ok(())
        },
        Ok(None) => Err(UnminificationError::NoUnminifiedBody.into()),
        Err(err) => Err(err)
    }
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

fn read_v1_2_har(path: &PathBuf) -> Result<Log> {
    match from_path(path)?.log {
        har::Spec::V1_2(log) => Ok(log),
        _ => Err(HarbingerError::UnsupportedHarVersion.into()),
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
