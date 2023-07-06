use anyhow::Result;
use indicatif::{ProgressBar, ProgressStyle};
use std::fs::{create_dir, create_dir_all, OpenOptions};
use std::io::Write;
use std::path::PathBuf;

use crate::error::HarbingerError;
use crate::har::Har;
use crate::js::{parse_swc_ast, unpack_webpack_chunk_list, write_script};

pub fn dump(har: &Har, output_path: &PathBuf, unminify: bool) -> Result<()> {
    if output_path.try_exists()? {
        return Err(HarbingerError::DumpPathExists.into());
    }
    create_dir(output_path)?;

    let pb_style =
        ProgressStyle::with_template("{prefix:.bold.dim} {spinner} {bar} {msg}").unwrap();
    let pb = ProgressBar::new(har.entries.len() as u64);
    pb.set_style(pb_style);

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
            let script = parse_swc_ast(path.to_string_lossy().to_string(), body_str.to_string())?;
            if let Some(chunks) = unpack_webpack_chunk_list(&script) {
                let mut unpack_path = path.with_extension("");
                let file_name = unpack_path.file_name().unwrap().to_str().unwrap();
                unpack_path.set_file_name(format!("{}_unbundled", file_name));
                pb.println(format!(
                    " * detected {} webpack chunks, unpacking to {}...",
                    chunks.len(),
                    unpack_path.display()
                ));
                create_dir(&unpack_path)?;
                for chunk in chunks {
                    pb.println(format!("  * unpacking {}...", chunk.label));
                    let mut chunk_path = unpack_path.join(&chunk.label);
                    chunk_path.set_extension("js");
                    write_script(&chunk.into_script(), &chunk_path)?;
                }
            }
            pb.println(" * unminifying...");
            write_script(&script, &path)?;
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
