#![feature(async_closure)]

use log::{error, info};
#[cfg(feature = "redis-ratelimiter")]
use spectacles_proxy::ratelimiter::redis::RedisRatelimiter;
#[cfg(feature = "metrics")]
use spectacles_proxy::runtime::metrics::start_server;
use spectacles_proxy::{
	ratelimiter::{local::LocalRatelimiter, Ratelimiter},
	runtime::{broker::Broker, Client, Config},
};
use std::sync::Arc;
use tokio::{
	spawn,
	time::{sleep, Duration},
};
use uriparse::Scheme;

#[tokio::main]
async fn main() {
	env_logger::init();

	let config = Config::from_toml_file("proxy.toml")
		.unwrap_or_default()
		.with_env();

	let broker = Broker::from_config(&config).await;

	let ratelimiter = get_ratelimiter(&config).await;
	let client = Arc::new(Client {
		http: reqwest::Client::new(),
		ratelimiter: Arc::new(ratelimiter),
		api_base: "discord.com",
		api_scheme: Scheme::HTTPS,
		api_version: config.discord.api_version,
		timeout: config.timeout.map(|d| d.into()),
	});

	#[cfg(feature = "metrics")]
	if let Some(config) = config.metrics {
		info!("Launching metrics server");
		spawn(start_server(config.path, config.addr));
	}

	info!("Beginning normal message consumption");
	broker
		.consume_messages(move |msg| {
			let client = Arc::clone(&client);
			async move { client.handle_message(msg).await }
		})
		.await;
}

#[cfg(feature = "redis-ratelimiter")]
async fn get_ratelimiter(config: &Config) -> Box<dyn Ratelimiter + Send + Sync + 'static> {
	match &config.redis {
		Some(config) => {
			let redis_client =
				redis::Client::open(config.url.to_string()).expect("Unable to connect to Redis");

			let ratelimiter = loop {
				match RedisRatelimiter::new(&redis_client).await {
					Ok(r) => break r,
					Err(e) => error!("Error setting up Redis ratelimiter; retrying in 5s: {}", e),
				}

				sleep(Duration::from_secs(5)).await;
			};

			Box::new(ratelimiter)
		}
		None => Box::new(LocalRatelimiter::default()),
	}
}

#[cfg(not(feature = "redis-ratelimiter"))]
async fn get_ratelimiter(_config: &Config) -> impl Ratelimiter {
	LocalRatelimiter::default()
}
