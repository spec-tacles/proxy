use anyhow::Result;
pub use reqwest;
use reqwest::{header::HeaderMap, Response};
use std::{future::Future, pin::Pin, str::FromStr, sync::Arc};

pub mod local;
#[cfg(feature = "redis-ratelimiter")]
pub mod redis;

pub type FutureResult<T> = Pin<Box<dyn Future<Output = Result<T>> + Send>>;

pub trait Ratelimiter {
	fn claim(self: Arc<Self>, bucket: String) -> FutureResult<()>;
	fn release(self: Arc<Self>, bucket: String, info: RatelimitInfo) -> FutureResult<()>;
}

#[derive(Debug, Default, Eq, PartialEq)]
pub struct RatelimitInfo {
	pub limit: Option<usize>,
	pub resets_in: Option<u64>,
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
						.map(|r: f64| (r * 1000.) as u64),
				}
			}
			Err(_) => Self::default(),
		}
	}
}

#[cfg(test)]
mod test {
	use super::{RatelimitInfo, Ratelimiter};
	use anyhow::{anyhow, Result};
	use std::{
		sync::{
			atomic::{AtomicBool, Ordering},
			Arc,
		},
		time::{Duration, SystemTime},
	};
	use tokio::{
		time::{delay_for, timeout},
		try_join,
	};

	pub fn setup() {
		let _ = env_logger::builder()
			.is_test(true)
			.format_timestamp_nanos()
			.filter_level(log::LevelFilter::Debug)
			.try_init();
	}

	async fn claim_timeout(
		client: Arc<impl Ratelimiter>,
		bucket: &str,
		min_millis: u64,
		max_millis: u64,
	) -> Result<()> {
		let min = Duration::from_millis(min_millis);
		let max = Duration::from_millis(max_millis);
		let start = SystemTime::now();
		timeout(max, client.claim(bucket.to_string())).await??;

		let end = SystemTime::now();
		if end < start + min {
			return Err(anyhow!(
				"failed to claim \"{}\" in more than {:?} (claimed in {:?})",
				bucket,
				min,
				end.duration_since(start)?,
			));
		}

		Ok(())
	}

	pub async fn claim_release(client: Arc<impl Ratelimiter>) -> Result<()> {
		claim_timeout(client.clone(), "foo", 0, 50).await?;

		let released = Arc::new(AtomicBool::new(false));
		try_join!(
			async {
				claim_timeout(client.clone(), "foo", 0, 100).await?;
				match released.load(Ordering::Relaxed) {
					true => Ok(()),
					false => Err(anyhow::anyhow!("claimed before lock was released")),
				}
			},
			async {
				client
					.clone()
					.release("foo".into(), RatelimitInfo::default())
					.await?;
				released.store(true, Ordering::Relaxed);
				Ok(())
			},
		)?;

		Ok(())
	}

	pub async fn claim_timeout_release(client: Arc<impl Ratelimiter>) -> Result<()> {
		claim_timeout(client.clone(), "foo", 0, 50).await?;

		let start = SystemTime::now();
		client
			.clone()
			.release(
				"foo".into(),
				RatelimitInfo {
					limit: None,
					resets_in: Some(5000),
				},
			)
			.await?;

		claim_timeout(client.clone(), "foo", 0, 50).await?;

		let min = Duration::from_secs(5) - SystemTime::now().duration_since(start)?;
		let min = min.as_millis() as u64;
		claim_timeout(client, "foo", min, min + 50).await?;
		Ok(())
	}

	pub async fn claim_3x(client: Arc<impl Ratelimiter>) -> Result<()> {
		try_join!(
			claim_timeout(client.clone(), "foo", 0, 50),
			claim_timeout(client.clone(), "foo", 5000, 5050),
			claim_timeout(client.clone(), "foo", 10000, 10050),
			async {
				for _ in 0..2 {
					tokio::time::delay_for(Duration::from_secs(5)).await;
					client
						.clone()
						.release(
							"foo".into(),
							RatelimitInfo {
								limit: None,
								resets_in: None,
							},
						)
						.await?;
				}

				Ok(())
			}
		)?;

		Ok(())
	}

	pub async fn claim_limit_release(client: Arc<impl Ratelimiter>) -> Result<()> {
		claim_timeout(client.clone(), "foo", 0, 50).await?;
		client
			.clone()
			.release(
				"foo".into(),
				RatelimitInfo {
					limit: Some(2),
					resets_in: None,
				},
			)
			.await?;

		for _ in 0..2 {
			claim_timeout(client.clone(), "foo", 0, 50).await?;
		}

		let released = AtomicBool::new(false);
		try_join!(
			async {
				claim_timeout(client.clone(), "foo", 5000, 5050).await?;
				match released.load(Ordering::Relaxed) {
					true => Ok(()),
					false => Err(anyhow::anyhow!("claimed before release")),
				}
			},
			async {
				delay_for(Duration::from_secs(5)).await;
				client
					.clone()
					.release(
						"foo".into(),
						RatelimitInfo {
							limit: Some(2),
							resets_in: None,
						},
					)
					.await?;
				released.store(true, Ordering::Relaxed);
				Ok(())
			}
		)?;

		Ok(())
	}

	pub async fn claim_limit_timeout(client: Arc<impl Ratelimiter>) -> Result<()> {
		claim_timeout(client.clone(), "foo", 0, 50).await?;

		let start = SystemTime::now();
		client
			.clone()
			.release(
				"foo".into(),
				RatelimitInfo {
					limit: Some(2),
					resets_in: Some(5000),
				},
			)
			.await?;

		for _ in 0..2 {
			claim_timeout(client.clone(), "foo", 0, 50).await?;
		}

		let min = Duration::from_secs(5) - SystemTime::now().duration_since(start)?;
		let min = min.as_millis() as u64;
		claim_timeout(client.clone(), "foo", min, min + 50).await?;
		claim_timeout(client, "foo", 0, 50).await?;

		Ok(())
	}

	pub async fn claim_limit_release_timeout(client: Arc<impl Ratelimiter>) -> Result<()> {
		claim_timeout(client.clone(), "foo", 0, 50).await?;

		let start = SystemTime::now();
		client
			.clone()
			.release(
				"foo".into(),
				RatelimitInfo {
					limit: Some(2),
					resets_in: Some(5000),
				},
			)
			.await?;

		for _ in 0..2 {
			claim_timeout(client.clone(), "foo", 0, 50).await?;
		}

		delay_for(Duration::from_secs(1)).await;
		client
			.clone()
			.release(
				"foo".into(),
				RatelimitInfo {
					limit: Some(2),
					resets_in: Some(4000),
				},
			)
			.await?;

		let min = Duration::from_secs(5) - SystemTime::now().duration_since(start)?;
		let min = min.as_millis() as u64;
		claim_timeout(client.clone(), "foo", min, min + 50).await?;
		claim_timeout(client.clone(), "foo", 0, 50).await?;

		Ok(())
	}
}
