use anyhow::Result;
#[cfg(not(feature = "redis-ratelimiter"))]
use spectacles_proxy::ratelimiter::local::LocalRatelimiter;
#[cfg(feature = "redis-ratelimiter")]
use spectacles_proxy::ratelimiter::redis::RedisRatelimiter;
#[cfg(feature = "metrics")]
use spectacles_proxy::runtime::metrics::start_server;
use spectacles_proxy::{
	ratelimiter::Ratelimiter,
	runtime::{Client, Config},
};
use tokio::spawn;
use tracing::info;
use tracing_subscriber::EnvFilter;
use uriparse::Scheme;

#[tokio::main]
async fn main() -> Result<()> {
	tracing_subscriber::fmt()
		.with_env_filter(EnvFilter::from_default_env())
		.init();

	let config = Config::from_toml_file("proxy.toml")
		.unwrap_or_default()
		.with_env();

	let broker = config.new_broker();

	let ratelimiter = get_ratelimiter(&config);
	let client = Client {
		http: reqwest::Client::new(),
		ratelimiter,
		api_base: "discord.com".to_string(),
		api_scheme: Scheme::HTTPS,
		api_version: config.discord.api_version,
		timeout: config.timeout.map(|d| d.into()),
	};

	#[cfg(feature = "metrics")]
	if let Some(ref config) = config.metrics {
		info!("Launching metrics server");
		spawn(start_server(config.path.clone(), config.addr));
	}

	let events = vec![config.broker.event.into()];
	broker.ensure_events(events.iter()).await?;

	info!("Beginning normal message consumption");
	client.consume_stream(broker.consume(events)).await?;

	Ok(())
}

#[cfg(feature = "redis-ratelimiter")]
fn get_ratelimiter(config: &Config) -> impl Ratelimiter + Clone {
	let manager = redust::pool::Manager::new(config.redis.url.clone());
	let pool = redust::pool::Pool::builder(manager)
		.max_size(config.redis.pool_size)
		.build()
		.expect("Unable to connect to Redis");

	RedisRatelimiter::new(pool.clone())
}

#[cfg(not(feature = "redis-ratelimiter"))]
fn get_ratelimiter(_config: &Config) -> impl Ratelimiter + Clone {
	LocalRatelimiter::default()
}
