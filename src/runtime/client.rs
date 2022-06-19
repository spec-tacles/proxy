#[cfg(feature = "metrics")]
use crate::metrics::{RATELIMIT_LATENCY, REQUESTS_TOTAL, REQUEST_LATENCY, RESPONSES_TOTAL};
use crate::{
	models::{RequestResponse, SerializableHttpRequest, SerializableHttpResponse},
	ratelimiter::Ratelimiter,
	route::make_route,
};
use anyhow::{Context, Result};
use futures::{TryStream, TryStreamExt};
use http::Method;
use reqwest::Request;
use rustacles_brokers::redis::message::Message;
use std::{convert::TryInto, fmt::Debug, str::FromStr, time::SystemTime};
use tokio::{
	net::ToSocketAddrs,
	spawn,
	time::{self, timeout_at, Duration, Instant},
};
use tracing::{info, instrument, warn};
use uriparse::{Path, Query, Scheme, URIBuilder};

#[cfg(feature = "metrics")]
use super::metrics::LatencyTracker;

#[derive(Debug, Clone)]
pub struct Client<R> {
	pub http: reqwest::Client,
	pub ratelimiter: R,
	pub api_scheme: Scheme<'static>,
	pub api_version: u8,
	pub api_base: String,
	pub timeout: Option<Duration>,
}

impl<R> Client<R>
where
	R: Ratelimiter + Clone + Sync + Send + 'static,
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
				(&*self.api_base)
					.try_into()
					.expect("Invalid authority configuration"),
			))
			.path(path);

		if let Some(query) = &data.query {
			let maybe_qs =
				query
					.iter()
					.map(|(k, v)| format!("{}={}", k, v))
					.reduce(|mut acc, pair| {
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

	#[instrument(level = "trace", skip(self), ret)]
	async fn claim(&self, data: &SerializableHttpRequest) -> Result<(Request, String)> {
		let req = self.create_request(data)?;
		let bucket = make_route(req.url().path())?;
		self.ratelimiter.claim(bucket.clone()).await?;

		Ok((req, bucket))
	}

	#[instrument(level = "debug", skip(self))]
	async fn do_request<A>(
		&self,
		message: &Message<A, SerializableHttpRequest>,
		data: &SerializableHttpRequest,
	) -> Result<SerializableHttpResponse>
	where
		A: ToSocketAddrs + Clone + Send + Sync + Debug,
	{
		#[cfg(feature = "metrics")]
		let req_labels: [&str; 2] = [&data.method, &data.path];

		let claim = {
			#[cfg(feature = "metrics")]
			let _ = LatencyTracker::new(&RATELIMIT_LATENCY, &req_labels);
			self.claim(data).await
		};

		message.ack().await?;
		let (req, bucket) = claim?;

		#[cfg(feature = "metrics")]
		REQUESTS_TOTAL.get_metric_with_label_values(&req_labels)?.inc();

		let res = {
			#[cfg(feature = "metrics")]
			let _ = LatencyTracker::new(&REQUEST_LATENCY, &req_labels);
			self.http.execute(req).await
		};

		self.ratelimiter
			.release(bucket, res.as_ref().into())
			.await?;
		let res = res?;

		#[cfg(feature = "metrics")]
		{
			let status = res.status();
			let res_labels = [&data.method, &data.path, status.as_str()];
			RESPONSES_TOTAL.get_metric_with_label_values(&res_labels)?.inc();
		}

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

	pub async fn consume_stream<A>(
		&self,
		mut stream: impl TryStream<
				Ok = Message<A, SerializableHttpRequest>,
				Error = rustacles_brokers::error::Error,
			> + Unpin,
	) -> Result<()>
	where
		A: 'static + ToSocketAddrs + Clone + Send + Sync + Debug,
	{
		while let Some(message) = stream.try_next().await? {
			let client = self.clone();
			match message.timeout_at {
				Some(timeout) => {
					let duration = timeout.duration_since(SystemTime::now()).expect("duration");
					let instant = Instant::now() + duration;
					spawn(async move {
						timeout_at(instant, client.handle_message(message)).await;
					});
				}
				None => {
					spawn(async move {
						client.handle_message(message).await;
					});
				}
			}
		}

		Ok(())
	}

	#[instrument(level = "debug", skip(self))]
	pub async fn handle_message<A>(
		&self,
		message: Message<A, SerializableHttpRequest>,
	) -> Result<()>
	where
		A: ToSocketAddrs + Clone + Send + Sync + Debug,
	{
		message.ack().await?;

		let data = match message.data {
			Some(ref data) => data,
			None => {
				warn!("Message missing data");
				return Ok(());
			}
		};
		info!("--> REQ({}): {}", message.id, data);

		let timeout = data.timeout;
		let req = self.do_request(&message, &data);

		let body = if let Some(min_timeout) = self.timeout.min(timeout) {
			time::timeout(min_timeout, req).await?
		} else {
			req.await
		};

		match &body {
			Ok(res) => info!("<-- RES({}): {}", message.id, res),
			Err(e) => warn!("<-- ERR({}): {:?}", message.id, e),
		}

		let body = RequestResponse::<SerializableHttpResponse>::from(body);

		message
			.reply(&body)
			.await
			.expect("Unable to respond to query");

		Ok(())
	}
}
