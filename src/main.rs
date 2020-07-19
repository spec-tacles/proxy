#![feature(iterator_fold_self)]

#[macro_use]
extern crate log;

use anyhow::{Context, Result};
use reqwest::{Client, Method};
use rustacles_brokers::amqp::{AmqpBroker, Delivery};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::HashMap, convert::TryInto, ops::Deref, str::FromStr, sync::Arc};
use uriparse::{Query, Scheme, URIBuilder};

mod config;
mod ratelimiter;

#[derive(Debug, Serialize, Deserialize)]
struct SerializableHttpRequest {
	method: String,
	path: String,
	query: Option<HashMap<String, String>>,
	body: Option<Value>,
	#[serde(default)]
	headers: HashMap<String, String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct SerializableHttpResponse {
	status: u16,
	headers: HashMap<String, String>,
	url: String,
	body: Value,
}

async fn handle_request(
	http: Arc<Client>,
	ratelimiter: Arc<impl ratelimiter::Ratelimiter>,
	broker: impl Deref<Target = AmqpBroker>,
	message: Delivery,
) -> Result<()> {
	let data = serde_json::from_slice::<SerializableHttpRequest>(&message.data)
		.context("Unable to deserialize request data")?;

	let full_path = format!("/api/v6/{}", data.path);
	let mut builder = URIBuilder::new();
	builder
		.scheme(Scheme::HTTPS)
		.authority(Some("discord.com".try_into()?))
		.path(full_path.as_str().try_into()?);

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

	let req = http
		.request(Method::from_str(&data.method)?, &url.to_string())
		.headers((&data.headers).try_into()?)
		.body(serde_json::to_vec(&data.body)?)
		.build()?;

	let res = ratelimiter.make_request(req).await?;

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
		body: res.json().await?,
	};

	broker
		.reply_to(&message, serde_json::to_vec(&res_data)?)
		.await?;
	Ok(())
}

#[tokio::main]
async fn main() {
	env_logger::init();

	let config = toml::from_slice::<config::Config>(
		&std::fs::read("proxy.toml").expect("Unable to read config file"),
	)
	.expect("Invalid config file")
	.with_env();

	let redis_client = redis::Client::open(config.redis.url).unwrap();
	let broker = AmqpBroker::new(&config.amqp.url, config.amqp.group, config.amqp.subgroup)
		.await
		.expect("Unable to start AMQP broker");
	let mut consumer = broker
		.consume(&config.amqp.event)
		.await
		.expect("Unable to begin message consumption");

	let redis_ratelimiter = ratelimiter::redis::RedisRatelimiter::new(&redis_client)
		.await
		.expect("Unable to create ratelimiter");

	let http = Arc::new(Client::new());
	let broker = Arc::new(broker);
	let redis = Arc::new(redis_ratelimiter);
	while let Some(delivery) = consumer.recv().await {
		tokio::spawn(handle_request(
			Arc::clone(&http),
			Arc::clone(&redis),
			Arc::clone(&broker),
			delivery,
		));
	}
}
