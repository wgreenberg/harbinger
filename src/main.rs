mod blackhole;
mod dump;
mod error;
mod har;
mod js;
mod server;

use clap::{Parser, Subcommand};
use std::path::PathBuf;
use tokio::join;

use crate::blackhole::build_blackhole;
use crate::dump::dump;
use crate::har::Har;
use crate::server::build_server;

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

        #[arg(long, short, default_value_t = 8000)]
        port: u16,

        #[arg(long)]
        proxy: Option<reqwest::Url>,

        #[arg(long)]
        blackhole_port: Option<u16>,
    },
    Dump {
        har_path: PathBuf,

        #[arg(long, short)]
        output_path: PathBuf,

        #[arg(long)]
        unminify: bool,
    },
}

#[rocket::main]
async fn main() {
    let args = Args::parse();
    let har = Har::read(args.get_path()).unwrap();
    match &args.command {
        Command::Serve {
            dump_path,
            port,
            proxy,
            blackhole_port,
            ..
        } => {
            let harbinger_server = build_server(&har, *port, dump_path, proxy)
                .expect("failed to initialize server from HAR");
            if let Some(port) = blackhole_port {
                let blackhole = build_blackhole(*port);
                let _ = join!(harbinger_server.launch(), blackhole.launch());
            } else {
                let _ = harbinger_server.launch().await;
            }
        }
        Command::Dump {
            output_path,
            unminify,
            ..
        } => {
            dump(&har, output_path, *unminify).expect("failed to dump HAR");
        }
    }
}
