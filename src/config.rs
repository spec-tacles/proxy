use serde::Deserialize;
use std::env;

#[derive(Debug, Default, Deserialize)]
pub struct Config {
	#[serde(default)]
	pub redis: RedisConfig,
	#[serde(default)]
	pub amqp: AmqpConfig,
}

impl Config {
	pub fn with_env(mut self) -> Self {
		for (k, v) in env::vars() {
			match k.as_str() {
				"REDIS_URL" => self.redis.url = v,
				"AMQP_URL" => self.amqp.url = v,
				"AMQP_GROUP" => self.amqp.group = v,
				"AMQP_SUBGROUP" => self.amqp.subgroup = Some(v),
				"AMQP_EVENT" => self.amqp.event = v,
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
}

impl Default for AmqpConfig {
	fn default() -> Self {
		Self {
			url: Self::default_url(),
			group: Self::default_group(),
			subgroup: None,
			event: Self::default_event(),
		}
	}
}
