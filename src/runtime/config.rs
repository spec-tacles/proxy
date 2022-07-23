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
	#[serde(with = "humantime_serde")]
	pub timeout: Option<Duration>,
	pub metrics: Option<MetricsConfig>,
	#[serde(default)]
	pub broker: BrokerConfig,
}

impl Config {
	pub fn from_toml_file(file: &str) -> Result<Self> {
		Ok(toml::from_slice(&std::fs::read(file)?)?)
	}

	pub fn with_env(mut self) -> Self {
		for (k, v) in env::vars() {
			match k.as_str() {
				"BROKER_GROUP" => self.broker.group = v,
				"BROKER_EVENT" => self.broker.event = v,
				"REDIS_URL" => self.redis.url = v,
				"REDIS_POOL_SIZE" => {
					self.redis.pool_size = v.parse().expect("valid REDIS_POOL_SIZE (usize)")
				}
				"TIMEOUT" => self.timeout = parse_duration(&v).ok(),
				"DISCORD_API_VERSION" => {
					self.discord.api_version = v.parse().expect("valid DISCORD_API_VERSION (u8)")
				}
				"METRICS_ADDR" => {
					self.metrics.get_or_insert(MetricsConfig::default()).addr =
						v.parse().expect("valid METRICS_ADDR (SocketAddr)")
				}
				"METRICS_PATH" => {
					self.metrics.get_or_insert(MetricsConfig::default()).path = v;
				}
				_ => {}
			}
		}

		self
	}

	pub fn new_broker(&self) -> RedisBroker<String> {
		let manager = Manager::new(self.redis.url.clone());
		let pool = Pool::builder(manager)
			.max_size(self.redis.pool_size)
			.build()
			.unwrap();

		RedisBroker::new(self.broker.group.clone(), pool)
	}
}

#[derive(Debug, Deserialize)]
pub struct RedisConfig {
	#[serde(default = "RedisConfig::default_url")]
	pub url: String,
	#[serde(default = "RedisConfig::default_pool_size")]
	pub pool_size: usize,
}

impl RedisConfig {
	fn default_url() -> String {
		"localhost:6379".into()
	}

	fn default_pool_size() -> usize {
		32
	}
}

impl Default for RedisConfig {
	fn default() -> Self {
		Self {
			url: Self::default_url(),
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
		return 10;
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

#[derive(Debug, Deserialize)]
pub struct BrokerConfig {
	#[serde(default = "BrokerConfig::default_group")]
	pub group: String,
	#[serde(default = "BrokerConfig::default_event")]
	pub event: String,
}

impl BrokerConfig {
	fn default_group() -> String {
		"proxy".to_string()
	}

	fn default_event() -> String {
		"REQUEST".to_string()
	}
}

impl Default for BrokerConfig {
	fn default() -> Self {
		Self {
			group: Self::default_group(),
			event: Self::default_event(),
		}
	}
}
