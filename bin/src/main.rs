#![feature(iterator_fold_self, option_zip)]

use anyhow::{Context, Result};
use rustacles_brokers::amqp::{AmqpBroker, Delivery};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use serde_repr::*;
use spectacles_proxy::ratelimiter::{
	redis::{redis, RedisRatelimiter},
	reqwest::{self, Method},
	Ratelimiter,
};
use std::{collections::HashMap, convert::TryInto, mem::drop, str::FromStr, sync::Arc};
use tokio::{
	select,
	signal::ctrl_c,
	sync::mpsc,
	time::{delay_for, timeout, Duration},
};
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
	timeout: Option<Duration>,
}

#[derive(Debug, Serialize, Deserialize, Eq, PartialEq)]
struct SerializableHttpResponse {
	status: u16,
	headers: HashMap<String, String>,
	url: String,
	body: Value,
}

#[repr(u8)]
#[derive(Debug, Serialize_repr, Deserialize_repr, Eq, PartialEq)]
enum ResponseStatus {
	Success,
	Unknown,
	InvalidRequestFormat,
	InvalidPath,
	InvalidQuery,
	InvalidMethod,
	InvalidHeaders,
	RequestFailure,
	RequestTimeout,
}

impl From<&(dyn std::error::Error + 'static)> for ResponseStatus {
	fn from(e: &(dyn std::error::Error + 'static)) -> Self {
		if e.is::<serde_json::Error>() {
			ResponseStatus::InvalidRequestFormat
		} else if e.is::<uriparse::PathError>() {
			ResponseStatus::InvalidPath
		} else if e.is::<uriparse::QueryError>() {
			ResponseStatus::InvalidQuery
		} else if e.is::<http::method::InvalidMethod>() {
			ResponseStatus::InvalidMethod
		} else if e.is::<http::Error>() {
			ResponseStatus::InvalidHeaders
		} else if e.is::<reqwest::Error>() {
			ResponseStatus::RequestFailure
		} else {
			ResponseStatus::Unknown
		}
	}
}

#[derive(Debug, Serialize, Deserialize, Eq, PartialEq)]
struct RequestResponse<T> {
	status: ResponseStatus,
	body: T,
}

impl<T> From<Result<T>> for RequestResponse<Value>
where
	T: Serialize,
{
	fn from(res: Result<T>) -> Self {
		match res {
			Err(e) => {
				let e_ref: &(dyn std::error::Error) = e.as_ref();
				Self {
					status: e_ref.into(),
					body: serde_json::to_value(e.to_string())
						.expect("Unable to serialize response error"),
				}
			}
			Ok(t) => Self {
				status: ResponseStatus::Success,
				body: serde_json::to_value(t).expect("Unable to serialize response data"),
			},
		}
	}
}

struct Client<'a, R> {
	http: Arc<reqwest::Client>,
	ratelimiter: Arc<R>,
	broker: Arc<AmqpBroker>,
	api_scheme: Scheme<'a>,
	api_version: u8,
	api_base: &'a str,
	timeout: Option<Duration>,
}

impl<'a, R> Client<'a, R>
where
	R: Ratelimiter + Sync + Send + 'static,
{
	async fn handle_request(&self, message: &Delivery) {
		let body: RequestResponse<Value> = self.handle_message(&message.data).await.into();

		self.broker
			.reply_to(
				message,
				serde_json::to_vec(&body).expect("Unable to serialize response body"),
			)
			.await
			.expect("Unable to respond to query");
	}

	async fn handle_message(&self, data: &[u8]) -> Result<SerializableHttpResponse> {
		let data = serde_json::from_slice::<SerializableHttpRequest>(&data)?;

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
			.authority(Some(
				self.api_base
					.try_into()
					.expect("Invalid authority configuration"),
			))
			.path(path);

		if let Some(query) = &data.query {
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

		let call = Arc::clone(&self.ratelimiter).make_request(Arc::clone(&self.http), http_req);

		let res = if let Some(min_timeout) = self.timeout.min(data.timeout) {
			timeout(min_timeout, call).await??
		} else {
			call.await?
		};

		Ok(SerializableHttpResponse {
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
		})
	}
}

#[tokio::main]
async fn main() {
	env_logger::init();

	let config = config::Config::from_toml_file("proxy.toml")
		.unwrap_or_default()
		.with_env();

	let redis_client = redis::Client::open(config.redis.url).expect("Unable to connect to Redis");
	let broker = Arc::new(loop {
		match AmqpBroker::new(
			&config.amqp.url,
			config.amqp.group.clone(),
			config.amqp.subgroup.clone(),
		)
		.await
		{
			Ok(b) => break b,
			Err(e) => eprintln!("Error connecting to AMQP; retrying in 5s: {}", e),
		}

		delay_for(Duration::from_secs(5)).await;
	});

	let mut consumer = broker
		.consume(&config.amqp.event)
		.await
		.expect("Unable to setup message consumption");

	let ratelimiter = loop {
		match RedisRatelimiter::new(&redis_client).await {
			Ok(r) => break r,
			Err(e) => eprintln!("Error setting up Redis ratelimiter; retrying in 5s: {}", e),
		}

		delay_for(Duration::from_secs(5)).await;
	};

	let client = Arc::new(Client {
		http: Arc::new(reqwest::Client::new()),
		broker: Arc::clone(&broker),
		ratelimiter: Arc::new(ratelimiter),
		api_base: "discord.com",
		api_scheme: Scheme::HTTPS,
		api_version: 6,
		timeout: None,
	});

	let (republish_send, mut republish_recv) = mpsc::unbounded_channel::<Vec<u8>>();

	println!("Beginning normal message consumption");
	while let Some(message) = select! {
		_ = ctrl_c() => None,
		m = consumer.recv() => m,
	} {
		let client = Arc::clone(&client);
		let republish_send = republish_send.clone();
		tokio::spawn(async move {
			select! {
				_ = ctrl_c() => {
					republish_send.send(message.data).expect("Unable to republish message on exit");
				},
				_ = client.handle_request(&message) => {}
			};
		});
	}

	drop(client);
	drop(consumer);
	drop(republish_send);

	println!("Flushing incomplete messages back to AMQP before exiting");
	while let Some(data) = republish_recv.recv().await {
		broker
			.publish("REQUEST", data, Default::default())
			.await
			.expect("Unable to republish message to AMQP");
	}
}
