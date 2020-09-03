use super::*;
use anyhow::{Context, Result};
use rustacles_brokers::amqp::Message;
use spectacles_proxy::{
	ratelimiter::{
		reqwest::{self, Method, Request},
		Ratelimiter,
	},
	route::make_route,
};
use std::{convert::TryInto, str::FromStr, sync::Arc};
use tokio::time::{self, Duration};
use uriparse::{Path, Query, Scheme, URIBuilder};

#[derive(Debug, Clone)]
pub struct Client<'a, R> {
	pub http: reqwest::Client,
	pub ratelimiter: Arc<R>,
	pub api_scheme: Scheme<'a>,
	pub api_version: u8,
	pub api_base: &'a str,
	pub timeout: Option<Duration>,
}

impl<'a, R> Client<'a, R>
where
	R: Ratelimiter + Sync + Send + 'static,
{
	fn create_request(&self, data: SerializableHttpRequest) -> Result<Request> {
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

		if let Some(body) = data.body {
			req_builder = req_builder.body(body);
		}

		Ok(req_builder
			.build()
			.context("Unable to build HTTP request")?)
	}

	async fn claim(&self, data: SerializableHttpRequest) -> Result<(Request, String)> {
		let req = self.create_request(data)?;
		let bucket = make_route(req.url().path())?;
		self.ratelimiter.clone().claim(bucket.clone()).await?;

		Ok((req, bucket))
	}

	async fn do_request(
		&self,
		message: &Message,
		data: SerializableHttpRequest,
	) -> Result<SerializableHttpResponse> {
		let claim = self.claim(data).await;
		message.ack().await?;
		let (req, bucket) = claim?;

		let res = self.http.execute(req).await;
		self.ratelimiter
			.clone()
			.release(bucket, res.as_ref().into())
			.await?;
		let res = res?;

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

	pub async fn handle_message(&self, message: &Message) -> Result<()> {
		let data = rmp_serde::from_slice::<SerializableHttpRequest>(&message.data)?;
		let timeout = data.timeout;
		let req = self.do_request(&message, data);

		let body: RequestResponse<SerializableHttpResponse> =
			if let Some(min_timeout) = self.timeout.min(timeout) {
				time::timeout(min_timeout, req).await?
			} else {
				req.await
			}
			.into();

		message
			.reply(rmp_serde::to_vec(&body).expect("Unable to serialize response body"))
			.await
			.expect("Unable to respond to query");

		Ok(())
	}
}
