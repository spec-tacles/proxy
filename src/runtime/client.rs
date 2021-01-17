#[cfg(feature = "metrics")]
use crate::metrics::{RATELIMIT_LATENCY, REQUESTS_TOTAL, REQUEST_LATENCY, RESPONSES_TOTAL};
use crate::{
	models::{RequestResponse, SerializableHttpRequest, SerializableHttpResponse},
	ratelimiter::Ratelimiter,
	route::make_route,
};
use anyhow::{Context, Result};
use http::Method;
use reqwest::Request;
use rmp_serde::Serializer;
use rustacles_brokers::amqp::Message;
use serde::Serialize;
use std::{convert::TryInto, ops::Deref, str::FromStr};
#[cfg(feature = "metrics")]
use tokio::time::Instant;
use tokio::time::{self, Duration};
use uriparse::{Path, Query, Scheme, URIBuilder};

#[derive(Debug, Clone)]
pub struct Client<'a, R> {
	pub http: reqwest::Client,
	pub ratelimiter: R,
	pub api_scheme: Scheme<'a>,
	pub api_version: u8,
	pub api_base: &'a str,
	pub timeout: Option<Duration>,
}

impl<'a, R, T> Client<'a, T>
where
	R: Ratelimiter + Sync + Send + 'static,
	T: Deref<Target = R>,
{
	fn create_request(&self, data: &SerializableHttpRequest) -> Result<Request> {
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

		let mut req_builder = self
			.http
			.request(Method::from_str(&data.method)?, &url.to_string())
			.headers((&data.headers).try_into()?);

		if let Some(body) = data.body.clone() {
			req_builder = req_builder.body(body);
		}

		Ok(req_builder
			.build()
			.context("Unable to build HTTP request")?)
	}

	async fn claim(&self, data: &SerializableHttpRequest) -> Result<(Request, String)> {
		let req = self.create_request(data)?;
		let bucket = make_route(req.url().path())?;
		self.ratelimiter.claim(bucket.clone()).await?;

		Ok((req, bucket))
	}

	async fn do_request(
		&self,
		message: &Message,
		data: &SerializableHttpRequest,
	) -> Result<SerializableHttpResponse> {
		#[cfg(feature = "metrics")]
		let start = Instant::now();
		let claim = self.claim(data).await;
		#[cfg(feature = "metrics")]
		let latency = Instant::now().duration_since(start);

		#[cfg(feature = "metrics")]
		let labels: [&str; 2] = [&data.method, &data.path];

		#[cfg(feature = "metrics")]
		RATELIMIT_LATENCY
			.get_metric_with_label_values(&labels)?
			.observe(latency.as_secs_f64());

		message.ack().await?;
		let (req, bucket) = claim?;

		#[cfg(feature = "metrics")]
		REQUESTS_TOTAL.get_metric_with_label_values(&labels)?.inc();

		#[cfg(feature = "metrics")]
		let start = Instant::now();
		let res = self.http.execute(req).await;
		#[cfg(feature = "metrics")]
		let latency = Instant::now().duration_since(start);

		self.ratelimiter
			.release(bucket, res.as_ref().into())
			.await?;
		let res = res?;

		#[cfg(feature = "metrics")]
		let status = res.status();
		#[cfg(feature = "metrics")]
		let labels = [&data.method, &data.path, status.as_str()];

		#[cfg(feature = "metrics")]
		RESPONSES_TOTAL.get_metric_with_label_values(&labels)?.inc();
		#[cfg(feature = "metrics")]
		REQUEST_LATENCY
			.get_metric_with_label_values(&labels)?
			.observe(latency.as_secs_f64());

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
			body: res.bytes().await?,
		})
	}

	pub async fn handle_message(&self, message: &Message) -> Result<()> {
		let data = match rmp_serde::from_slice::<SerializableHttpRequest>(&message.data) {
			Ok(data) => data,
			Err(e) => {
				message.ack().await?;
				return Err(e.into());
			}
		};
		let timeout = data.timeout;
		let req = self.do_request(&message, &data);

		let body: RequestResponse<SerializableHttpResponse> =
			if let Some(min_timeout) = self.timeout.min(timeout) {
				time::timeout(min_timeout, req).await?
			} else {
				req.await
			}
			.into();

		let mut buf = Vec::new();
		body.serialize(&mut Serializer::new(&mut buf).with_struct_map())
			.expect("Unable to serialize response body");

		message
			.reply(buf)
			.await
			.expect("Unable to respond to query");

		Ok(())
	}
}
