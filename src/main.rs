mod blackhole;
mod dump;
mod error;
mod guide;
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

        #[arg(long)]
        raw: bool,

        #[arg(long, short)]
        output_path: PathBuf,
    },
    Guide,
}

#[rocket::main]
async fn main() {
    let args = Args::parse();
    match &args.command {
        Command::Serve {
            har_path,
            dump_path,
            port,
            proxy,
            blackhole_port,
            ..
        } => {
            let har = Har::read(har_path).unwrap();
            let harbinger_server = build_server(&har, *port, dump_path.as_ref(), proxy.as_ref())
                .expect("failed to initialize server from HAR");
            if let Some(port) = blackhole_port {
                let blackhole = build_blackhole(*port);
                let _ = join!(harbinger_server.launch(), blackhole.launch());
            } else {
                let _ = harbinger_server.launch().await;
            }
        }
        Command::Dump {
            har_path,
            output_path,
            raw,
            ..
        } => {
            let har = Har::read(har_path).unwrap();
            match dump(&har, output_path, *raw) {
                Ok(_) => println!("Dumped HAR to {}", output_path.display()),
                Err(e) => println!("Failed to dump HAR: {}", e),
            }
        },
        Command::Guide => {
            guide::run().await;
        }
    }
}
