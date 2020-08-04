use super::*;
use anyhow::{Context, Result};
use rustacles_brokers::amqp::{AmqpBroker, Delivery};
use serde_json::Value;
use spectacles_proxy::ratelimiter::{
	reqwest::{self, Method},
	Ratelimiter,
};
use std::{convert::TryInto, str::FromStr, sync::Arc};
use tokio::time::{timeout, Duration};
use uriparse::{Path, Query, Scheme, URIBuilder};

pub struct Client<'a, R> {
	pub http: Arc<reqwest::Client>,
	pub ratelimiter: Arc<R>,
	pub broker: Arc<AmqpBroker>,
	pub api_scheme: Scheme<'a>,
	pub api_version: u8,
	pub api_base: &'a str,
	pub timeout: Option<Duration>,
}

impl<'a, R> Client<'a, R>
where
	R: Ratelimiter + Sync + Send + 'static,
{
	pub async fn handle_request(&self, message: &Delivery) {
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
