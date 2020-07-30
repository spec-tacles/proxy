#![feature(iterator_fold_self)]

use anyhow::{Context, Result};
use rustacles_brokers::amqp::{AmqpBroker, Delivery};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use spectacles_proxy::ratelimiter::{
	redis::{redis, RedisRatelimiter},
	reqwest::{self, Method},
	Ratelimiter,
};
use std::{collections::HashMap, convert::TryInto, str::FromStr, sync::Arc};
use uriparse::{Path, Query, Scheme, URIBuilder};

mod config;
#[cfg(test)]
mod test;

#[derive(Debug, Serialize, Deserialize, Eq, PartialEq)]
struct SerializableHttpRequest {
	method: String,
	path: String,
	query: Option<HashMap<String, String>>,
	body: Option<Value>,
	#[serde(default)]
	headers: HashMap<String, String>,
}

#[derive(Debug, Serialize, Deserialize, Eq, PartialEq)]
struct SerializableHttpResponse {
	status: u16,
	headers: HashMap<String, String>,
	url: String,
	body: Value,
}

struct Client<'a, R> {
	http: Arc<reqwest::Client>,
	ratelimiter: Arc<R>,
	broker: AmqpBroker,
	api_scheme: Scheme<'a>,
	api_version: u8,
	api_base: &'a str,
}

impl<'a, R> Client<'a, R>
where
	R: Ratelimiter + Sync + Send + 'static,
{
	async fn handle_request(&self, message: Delivery) -> Result<()> {
		let data = serde_json::from_slice::<SerializableHttpRequest>(&message.data)
			.context("Unable to deserialize request data")?;

		let path_str = format!(
			"/api/v{}/{}",
			self.api_version,
			data.path.strip_prefix('/').unwrap_or_default()
		);
		let mut path: Path = path_str.as_str().try_into()?;
		path.normalize(false);

		let mut builder = URIBuilder::new();
		builder
			.scheme(self.api_scheme.clone())
			.authority(Some(self.api_base.try_into()?))
			.path(path);

		if let Some(query) = data.query {
			let maybe_qs = query
				.iter()
				.map(|(k, v)| format!("{}={}", k, v))
				.fold_first(|mut acc, pair| {
					acc.push('&');
					acc.push_str(&pair);
					acc
				});

			if let Some(qs) = maybe_qs {
				let mut query: Query = qs.as_str().try_into()?;
				query.normalize();
				builder.query(Some(query.into_owned()));
			}
		}

		let url = builder.build()?;

		let http_req = self
			.http
			.request(Method::from_str(&data.method)?, &url.to_string())
			.headers((&data.headers).try_into()?)
			.body(serde_json::to_vec(&data.body)?)
			.build()
			.context("Unable to build HTTP request")?;

		let res = Arc::clone(&self.ratelimiter)
			.make_request(Arc::clone(&self.http), http_req)
			.await?
			.context("Unable to perform HTTP request")?;

		let res_data = SerializableHttpResponse {
			status: res.status().as_u16(),
			headers: res
				.headers()
				.into_iter()
				.map(|(name, value)| {
					(
						name.as_str().to_string(),
						value.to_str().unwrap().to_string(),
					)
				})
				.collect(),
			url: res.url().to_string(),
			body: res.json().await.context("Invalid JSON response")?,
		};

		self.broker
			.reply_to(&message, serde_json::to_vec(&res_data)?)
			.await
			.context("Unable to send response message")?;
		Ok(())
	}
}

#[tokio::main]
async fn main() {
	env_logger::init();

	let config = config::Config::from_toml_file("proxy.toml")
		.unwrap_or_default()
		.with_env();

	let redis_client = redis::Client::open(config.redis.url).unwrap();
	let broker = AmqpBroker::new(&config.amqp.url, config.amqp.group, config.amqp.subgroup)
		.await
		.expect("Unable to start AMQP broker");
	let mut consumer = broker
		.consume(&config.amqp.event)
		.await
		.expect("Unable to begin message consumption");

	let ratelimiter = RedisRatelimiter::new(&redis_client)
		.await
		.expect("Unable to create ratelimiter");

	let client = Arc::new(Client {
		http: Arc::new(reqwest::Client::new()),
		broker,
		ratelimiter: Arc::new(ratelimiter),
		api_base: "discord.com",
		api_scheme: Scheme::HTTPS,
		api_version: 6,
	});

	while let Some(message) = consumer.recv().await {
		let client = Arc::clone(&client);
		tokio::spawn(async move {
			let res = client.handle_request(message).await;
			if let Err(err) = res {
				eprintln!("Error handling message: {}", err);
			}
		});
	}
}
