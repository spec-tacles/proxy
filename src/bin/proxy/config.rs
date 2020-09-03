use anyhow::Result;
use humantime::Duration;
use serde::{
	de::{Deserializer, Visitor},
	Deserialize,
};
use std::{env, fmt, str::FromStr};

#[derive(Debug, Default, Deserialize)]
pub struct Config {
	#[serde(default)]
	pub redis: RedisConfig,
	#[serde(default)]
	pub amqp: AmqpConfig,
	#[serde(default, deserialize_with = "deserialize_duration")]
	pub timeout: Option<Duration>,
}

fn deserialize_duration<'de, D>(deserializer: D) -> Result<Option<Duration>, D::Error>
where
	D: Deserializer<'de>,
{
	struct DurationVisitor;

	impl<'de> Visitor<'de> for DurationVisitor {
		type Value = Option<Duration>;

		fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
			write!(f, "string")
		}

		fn visit_str<E>(self, value: &str) -> Result<Option<Duration>, E> {
			Ok(Some(Duration::from_str(value).unwrap()))
		}

		fn visit_none<E>(self) -> Result<Option<Duration>, E> {
			Ok(None)
		}
	}

	deserializer.deserialize_any(DurationVisitor {})
}

impl Config {
	pub fn from_toml_file(file: &str) -> Result<Self> {
		Ok(toml::from_slice(&std::fs::read(file)?)?)
	}

	pub fn with_env(mut self) -> Self {
		for (k, v) in env::vars() {
			match k.as_str() {
				"REDIS_URL" => self.redis.url = v,
				"AMQP_URL" => self.amqp.url = v,
				"AMQP_GROUP" => self.amqp.group = v,
				"AMQP_SUBGROUP" => self.amqp.subgroup = Some(v),
				"AMQP_EVENT" => self.amqp.event = v,
				"AMQP_CANCELLATION_EVENT" => self.amqp.cancellation_event = v,
				"TIMEOUT" => self.timeout = v.parse().ok(),
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
