use anyhow::Result;
pub use reqwest;
use reqwest::{header::HeaderMap, Client, Request, Response};
use std::{convert::TryFrom, future::Future, pin::Pin, str::FromStr, sync::Arc};
use tokio::task::{self, JoinHandle};
use uriparse::path::{Path, Segment};

pub mod local;
#[cfg(feature = "redis-ratelimiter")]
pub mod redis;

pub type FutureResult<T> = Pin<Box<dyn Future<Output = Result<T>> + Send>>;

pub trait Ratelimiter {
	fn claim(self: Arc<Self>, bucket: String) -> FutureResult<()>;
	fn release(self: Arc<Self>, bucket: String, info: RatelimitInfo) -> FutureResult<()>;

	fn make_request(
		self: Arc<Self>,
		client: Arc<Client>,
		req: Request,
	) -> JoinHandle<Result<Response>>
	where
		Self: Send + Sync + 'static,
	{
		let this = Arc::clone(&self);
		task::spawn(async move {
			let bucket = make_route(req.url().path())?;
			this.clone().claim(bucket.clone()).await?;
			let result = client.execute(req).await;
			this.release(bucket, result.as_ref().into()).await?;
			Ok(result?)
		})
	}
}

#[derive(Debug, Default)]
pub struct RatelimitInfo {
	pub limit: Option<usize>,
	pub resets_in: Option<u128>,
}

fn get_header<T: FromStr>(headers: &HeaderMap, key: &str) -> Option<T> {
	headers
		.get(key)
		.and_then(|value| Some(value.to_str().ok()?.parse().ok()?))
}

impl<'a, E> From<std::result::Result<&'a Response, E>> for RatelimitInfo {
	fn from(r: std::result::Result<&'a Response, E>) -> Self {
		match r {
			Ok(r) => {
				let headers = r.headers();
				Self {
					limit: get_header(headers, "x-ratelimit-limit"),
					resets_in: get_header(headers, "x-ratelimit-reset-after")
						.map(|r: f64| (r * 1000.) as u128),
				}
			}
			Err(_) => Self::default(),
		}
	}
}

#[derive(Debug)]
pub enum RouteError {
	RelativePath,
}

impl std::fmt::Display for RouteError {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::RelativePath => f.write_str("path is not absolute"),
		}
	}
}

impl std::error::Error for RouteError {}

pub fn make_route(path: &str) -> Result<String> {
	let mut path = Path::try_from(path)?;
	if !path.is_absolute() {
		return Err(RouteError::RelativePath.into());
	}

	let segments = path.segments_mut();
	match segments[0].as_str() {
		"guilds" | "channels" | "webhooks" if segments.len() > 1 => {
			segments[1] = Segment::try_from(":id")?;
			Ok(path.into())
		}
		_ => Ok(path.into()),
	}
}

#[cfg(test)]
mod test {
	use super::make_route;

	#[test]
	fn makes_route() {
		assert_eq!(make_route("/foo/bar").unwrap(), "/foo/bar".to_string());
		assert_eq!(
			make_route("/guilds/1234/roles").unwrap(),
			"/guilds/:id/roles".to_string()
		);
	}
}
