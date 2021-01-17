use anyhow::Result;
use humantime::parse_duration;
use serde::Deserialize;
use std::{env, net::SocketAddr, time::Duration};

#[derive(Debug, Default, Deserialize)]
pub struct Config {
	pub redis: Option<RedisConfig>,
	#[serde(default)]
	pub amqp: AmqpConfig,
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
				"REDIS_URL" => {
					self.redis.as_mut().map(|conf| conf.url = v);
				}
				"AMQP_URL" => self.amqp.url = v,
				"AMQP_GROUP" => self.amqp.group = v,
				"AMQP_SUBGROUP" => self.amqp.subgroup = Some(v),
				"AMQP_EVENT" => self.amqp.event = v,
				"AMQP_CANCELLATION_EVENT" => self.amqp.cancellation_event = v,
				"TIMEOUT" => self.timeout = parse_duration(&v).ok(),
				"DISCORD_API_VERSION" => {
					self.discord.api_version = v.parse().expect("valid DISCORD_API_VERSION (u8)")
				}
				_ => {}
			}
		}

		self
	}
}

#[derive(Debug, Deserialize)]
pub struct RedisConfig {
	#[serde(default = "RedisConfig::default_url")]
	pub url: String,
}

impl RedisConfig {
	fn default_url() -> String {
		"redis://localhost:6379".into()
	}
}

impl Default for RedisConfig {
	fn default() -> Self {
		Self {
			url: Self::default_url(),
		}
	}
}

#[derive(Debug, Deserialize)]
pub struct AmqpConfig {
	#[serde(default = "AmqpConfig::default_url")]
	pub url: String,
	#[serde(default = "AmqpConfig::default_group")]
	pub group: String,
	#[serde(default)]
	pub subgroup: Option<String>,
	#[serde(default = "AmqpConfig::default_event")]
	pub event: String,
	#[serde(default = "AmqpConfig::default_cancellation_event")]
	pub cancellation_event: String,
}

impl AmqpConfig {
	fn default_url() -> String {
		"amqp://localhost:5672/%2f".into()
	}

	fn default_group() -> String {
		"rest".into()
	}

	fn default_event() -> String {
		"REQUEST".into()
	}

	fn default_cancellation_event() -> String {
		"CANCEL".into()
	}
}

impl Default for AmqpConfig {
	fn default() -> Self {
		Self {
			url: Self::default_url(),
			group: Self::default_group(),
			subgroup: None,
			event: Self::default_event(),
			cancellation_event: Self::default_cancellation_event(),
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
