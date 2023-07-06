use rocket::{config::Config as RocketConfig, Build, Rocket};

pub fn build_blackhole(port: u16) -> Rocket<Build> {
    let server_config = RocketConfig::figment()
        .merge(("port", port))
        .merge(("log_level", "debug"));

    rocket::custom(server_config)
}
