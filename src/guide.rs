use std::path::{PathBuf, Path};

use tokio::join;

use crate::{Command, har::Har, dump::dump, server::build_server, blackhole::build_blackhole};

fn prompt_yes_or_no() -> Option<bool> {
    let mut response = String::new();
    std::io::stdin().read_line(&mut response).unwrap();
    let response = response.trim().to_lowercase();
    match response.as_str() {
        "yes" | "y" => Some(true),
        "no" | "n" => Some(false),
        _ => None,
    }
}

fn har_guide() -> Har {
    println!("Once you have your HAR file, please enter the path");
    println!("(e.g. /home/user/har.json):");
    let mut har_path = String::new();
    std::io::stdin().read_line(&mut har_path).unwrap();
    let har_path = PathBuf::from(har_path.trim());
    let har = match Har::read(&har_path) {
        Ok(har) => har,
        Err(e) => {
            println!("Error reading HAR file: {}", e);
            std::process::exit(1);
        }
    };
    println!();
    println!("Got HAR for url {} ({} entries)", har.primary_url(), har.entries.len());
    har
}

fn dump_guide(har: &Har) -> Option<PathBuf> {
    println!("Harbinger will now dump the HAR file to disk, unminifying any javascript it finds.");
    println!("Where would you like to dump the HAR file? (e.g. /home/user/dump):");
    let mut dump_path = String::new();
    std::io::stdin().read_line(&mut dump_path).unwrap();
    let dump_path = Path::new(dump_path.trim()).to_path_buf();
    println!("Dumping HAR to {}", dump_path.display());
    dump(&har, &dump_path, false).unwrap();
    Some(dump_path)
}

async fn server_guide(har: &Har, dump_path: Option<PathBuf>) {
    println!("Would you like to serve the HAR file? (y/n):");
    match prompt_yes_or_no() {
        Some(true) => {},
        Some(false) => return,
        _ => {
            println!("Invalid response");
            std::process::exit(1);
        }
    };

    println!();
    println!("What port would you like to serve on? (Default 8000):");
    let mut port = String::new();
    std::io::stdin().read_line(&mut port).unwrap();
    let port = port.trim().parse::<u16>().unwrap_or(8000);

    println!();
    println!("Harbinger provides a blackhole server which can be used to prevent requests from leaving your network.");
    println!("What port would you like to use for the blackhole server? (Default 8001):");
    let mut blackhole_port = String::new();
    std::io::stdin().read_line(&mut blackhole_port).unwrap();
    let blackhole_port = blackhole_port.trim().parse::<u16>().unwrap_or(8001);

    println!();
    println!("Would you like to proxy requests to another server? This is an advanced feature useful for serving dynamic content not present in the HAR.");
    println!("(y/n):");
    let proxy_server = match prompt_yes_or_no() {
        Some(true) => {
            println!("Please enter the full URL of the proxy server (including http:// or https://)");
            println!("(e.g. http://localhost:8001):");
            let mut proxy_server = String::new();
            std::io::stdin().read_line(&mut proxy_server).unwrap();
            let proxy_server = reqwest::Url::parse(proxy_server.trim()).unwrap();
            Some(proxy_server)
        },
        Some(false) => None,
        _ => {
            println!("Invalid response");
            std::process::exit(1);
        }
    };

    println!();
    println!("To utilize the blackhole server, and thus prevent requests from leaving your network, you'll need to configure your browser to use it as a proxy.");
    println!("This can be done by launching your browser from the command line like this:");
    println!("  google-chrome --proxy-server=http://localhost:{} --proxy-bypass-list=localhost", blackhole_port);
    println!("Once you've launched your browser, navigate to http://localhost:{}/harbinger to activate Harbinger's service worker. Press enter once you've done this.", port);
    std::io::stdin().read_line(&mut String::new()).unwrap();
    
    println!();
    println!("Starting the server...");
    let harbinger_server = build_server(&har, port, dump_path.as_ref(), proxy_server.as_ref())
        .expect("failed to initialize server from HAR");
    let blackhole = build_blackhole(port);
    let _ = join!(harbinger_server.launch(), blackhole.launch());
}

pub async fn run() {
    println!("Welcome to Harbinger! Let's get started.");
    println!();
    println!("First, you'll need a HAR file of the site you're analyzing. You can generate one using your browser's developer tools.");
    println!("If you're using Firefox, you can find instructions here: https://developer.mozilla.org/en-US/docs/Tools/Network_Monitor");
    println!("If you're using Chrome, you can find instructions here: https://developers.google.com/web/tools/chrome-devtools/network/reference#har-files");
    println!();
    let har = har_guide();
    println!();
    let dump_path = dump_guide(&har);
    println!();
    server_guide(&har, dump_path).await;
}
