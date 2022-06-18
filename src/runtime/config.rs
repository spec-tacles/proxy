use anyhow::Result;
use humantime::parse_duration;
use rustacles_brokers::redis::{
	redust::pool::{Manager, Pool},
	RedisBroker,
};
use serde::Deserialize;
use std::{env, net::SocketAddr, time::Duration};

#[derive(Debug, Default, Deserialize)]
pub struct Config {
	#[serde(default)]
	pub redis: RedisConfig,
	#[serde(default)]
	pub discord: DiscordConfig,
	#[serde(default, with = "humantime_serde")]
	pub timeout: Option<Duration>,
	pub metrics: Option<MetricsConfig>,
}

impl Config {
	pub fn from_toml_file(file: &str) -> Result<Self> {
		Ok(toml::from_slice(&std::fs::read(file)?)?)
	}

	pub fn with_env(mut self) -> Self {
		for (k, v) in env::vars() {
			match k.as_str() {
				"REDIS_URL" => self.redis.url = v,
				"REDIS_GROUP" => self.redis.group = v,
				"AMQP_EVENT" => self.redis.event = v,
				"TIMEOUT" => self.timeout = parse_duration(&v).ok(),
				"DISCORD_API_VERSION" => {
					self.discord.api_version = v.parse().expect("valid DISCORD_API_VERSION (u8)")
				}
				_ => {}
			}
		}

		self
	}

	pub fn new_broker(&self) -> RedisBroker<String> {
		let manager = Manager::new(self.redis.url.clone().parse().unwrap());
		let pool = Pool::builder(manager)
			.max_size(self.redis.pool_size)
			.build()
			.unwrap();
		let broker = RedisBroker::new(self.redis.group.clone(), pool);

		broker
	}
}

#[derive(Debug, Deserialize)]
pub struct RedisConfig {
	#[serde(default = "RedisConfig::default_url")]
	pub url: String,
	#[serde(default = "RedisConfig::default_group")]
	pub group: String,
	#[serde(default = "RedisConfig::default_event")]
	pub event: String,
	#[serde(default = "RedisConfig::default_pool_size")]
	pub pool_size: usize,
}

impl RedisConfig {
	fn default_url() -> String {
		"localhost:6379".into()
	}

	fn default_group() -> String {
		"gateway".into()
	}

	fn default_event() -> String {
		"REQUEST".into()
	}

	fn default_pool_size() -> usize {
		32
	}
}

impl Default for RedisConfig {
	fn default() -> Self {
		Self {
			url: Self::default_url(),
			group: Self::default_group(),
			event: Self::default_event(),
			pool_size: Self::default_pool_size(),
		}
	}
}

#[derive(Debug, Deserialize)]
pub struct DiscordConfig {
	#[serde(default = "DiscordConfig::default_api_version")]
	pub api_version: u8,
}

impl DiscordConfig {
	fn default_api_version() -> u8 {
		return 6;
	}
}

impl Default for DiscordConfig {
	fn default() -> Self {
		Self {
			api_version: Self::default_api_version(),
		}
	}
}

#[derive(Debug, Deserialize)]
pub struct MetricsConfig {
	#[serde(default = "MetricsConfig::default_addr")]
	pub addr: SocketAddr,
	#[serde(default = "MetricsConfig::default_path")]
	pub path: String,
}

impl MetricsConfig {
	fn default_addr() -> SocketAddr {
		([0, 0, 0, 0], 3000).into()
	}

	fn default_path() -> String {
		"metrics".to_owned()
	}
}

impl Default for MetricsConfig {
	fn default() -> Self {
		Self {
			addr: Self::default_addr(),
			path: Self::default_path(),
		}
	}
}
