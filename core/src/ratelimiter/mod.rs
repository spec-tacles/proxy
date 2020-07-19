use anyhow::Result;
pub use reqwest;
use reqwest::{header::HeaderMap, Request, Response};
use std::{convert::TryFrom, future::Future, pin::Pin, str::FromStr, sync::Arc};
use uriparse::path::{Path, Segment};

#[cfg(feature = "redis-ratelimiter")]
pub mod redis;

pub trait Ratelimiter {
	fn make_request(
		self: Arc<Self>,
		req: Request,
	) -> Pin<Box<dyn Future<Output = Result<Response>> + Send>>;
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
