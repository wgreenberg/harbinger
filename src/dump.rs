use anyhow::Result;
use deno_ast::ParsedSource;
use dprint_plugin_typescript::configuration::{
    ConfigurationBuilder, NextControlFlowPosition, QuoteStyle, UseParentheses,
};
use dprint_plugin_typescript::{configuration::Configuration, format_parsed_source};
use indicatif::{ProgressBar, ProgressStyle};
use std::fs::{create_dir, create_dir_all, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use thiserror::Error;

use crate::error::HarbingerError;
use crate::har::Har;
use crate::js::{parse_swc_ast, unpack_webpack_chunk_list};

pub fn dump(har: &Har, output_path: &PathBuf, unminify: bool) -> Result<()> {
    if output_path.try_exists()? {
        return Err(HarbingerError::DumpPathExists.into());
    }
    create_dir(output_path)?;

    let pb_style =
        ProgressStyle::with_template("{prefix:.bold.dim} {spinner} {bar} {msg}").unwrap();
    let pb = ProgressBar::new(har.entries.len() as u64);
    pb.set_style(pb_style);

    let unminification_config = ConfigurationBuilder::new()
        .line_width(80)
        .arrow_function_use_parentheses(UseParentheses::Force)
        .prefer_single_line(false)
        .quote_style(QuoteStyle::PreferSingle)
        .next_control_flow_position(NextControlFlowPosition::SameLine)
        .build();

    for (i, entry) in har.entries.iter().enumerate() {
        let uri = entry.uri()?;
        pb.set_prefix(format!("[{}/{}]", i, har.entries.len()));
        pb.set_message(format!("processing {}", uri));
        if entry.method() != "GET" {
            pb.println(format!("skipping {} {}", entry.method(), uri));
            continue;
        }

        let path = entry.get_dump_path(output_path);
        if let Some(parent_path) = path.parent() {
            create_dir_all(parent_path)?;
        }

        pb.println(format!("processing {}", uri));
        let body_bytes = entry.res_body().unwrap();
        if unminify && entry.res_header("content-type") == Some("application/javascript") {
            pb.println(" * parsing...");
            let body_str = std::str::from_utf8(&body_bytes).unwrap();
            let parsed_source = parse_swc_ast(&path, &body_str)?;
            if let Some(chunks) = unpack_webpack_chunk_list(&parsed_source) {
                let mut unpack_path = path.with_extension("");
                let file_name = unpack_path.file_name().unwrap().to_str().unwrap();
                unpack_path.set_file_name(format!("{}_unbundled", file_name));
                pb.println(format!(
                    " * detected {} webpack chunks, unpacking to {}...",
                    chunks.len(),
                    unpack_path.display()
                ));
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
            let mut file = OpenOptions::new().write(true).create(true).open(&path)?;
            file.write_all(&body_bytes)?;
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

fn write_unminified(path: &Path, source: &ParsedSource, config: &Configuration) -> Result<()> {
    let mut file = OpenOptions::new().write(true).create(true).open(path)?;
    match format_parsed_source(source, config) {
        Ok(Some(unminified_body)) => {
            file.write_all(unminified_body.as_bytes())?;
            Ok(())
        }
        Ok(None) => Err(UnminificationError::NoUnminifiedBody.into()),
        Err(err) => Err(err),
    }
}
