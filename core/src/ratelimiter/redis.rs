use super::{FutureResult, RatelimitInfo, Ratelimiter};
use anyhow::Result;
pub use redis;
use redis::Script;
use std::{sync::Arc, time::Duration};
use tokio::{
	stream::StreamExt,
	sync::{broadcast, Mutex},
};

static NOTIFY_KEY: &'static str = "rest_ready";

pub struct RedisRatelimiter {
	redis: Mutex<redis::aio::Connection>,
	claim_script: Script,
	release_script: Script,
	ready_publisher: broadcast::Sender<String>,
}

impl RedisRatelimiter {
	pub async fn new(redis: &redis::Client) -> Result<Self> {
		let pubsub = redis.get_async_connection().await?.into_pubsub();
		let main = redis.get_async_connection().await?;
		Self::new_from_connections(main, pubsub).await
	}

	pub async fn new_from_connections(
		main: redis::aio::Connection,
		mut pubsub: redis::aio::PubSub,
	) -> Result<Self> {
		pubsub.subscribe(NOTIFY_KEY).await?;
		let (sender, _) = broadcast::channel(32);

		let pubsub_sender = sender.clone();
		tokio::task::spawn(async move {
			while let Some(msg) = pubsub.on_message().next().await {
				let _ = pubsub_sender.send(msg.get_payload().unwrap());
			}
		});

		Ok(Self {
			redis: Mutex::new(main),
			claim_script: Script::new(include_str!("./scripts/claim.lua")),
			release_script: Script::new(include_str!("./scripts/release.lua")),
			ready_publisher: sender,
		})
	}
}

impl Ratelimiter for RedisRatelimiter {
	fn claim(self: Arc<Self>, bucket: String) -> FutureResult<()> {
		Box::pin(async move {
			'outer: loop {
				let expiration: isize = self
					.claim_script
					.key(&bucket)
					.key(bucket.to_string() + "_size")
					.invoke_async(&mut *self.redis.lock().await)
					.await?;

				debug!("Received expiration of {}ms for \"{}\"", expiration, bucket);

				if expiration.is_positive() {
					tokio::time::delay_for(Duration::from_millis(expiration as u64)).await;
					continue;
				}

				if expiration == 0 {
					break;
				}

				let mut rcv = self.ready_publisher.subscribe();
				loop {
					let opened_bucket = rcv.recv().await?;
					if opened_bucket == bucket {
						continue 'outer;
					}
				}
			}

			Ok(())
		})
	}

	fn release(self: Arc<Self>, bucket: String, info: RatelimitInfo) -> FutureResult<()> {
		Box::pin(async move {
			self.release_script
				.key(&bucket)
				.key(bucket.to_string() + "_size")
				.key(NOTIFY_KEY)
				.arg(info.limit.unwrap_or(0))
				.arg(info.resets_in.unwrap_or(0))
				.invoke_async(&mut *self.redis.lock().await)
				.await?;

			Ok(())
		})
	}
}

#[cfg(test)]
mod test {
	use super::{
		super::{RatelimitInfo, Ratelimiter},
		RedisRatelimiter,
	};
	use anyhow::{anyhow, Result};
	use redis::Client;
	use std::{
		sync::{
			atomic::{AtomicBool, AtomicUsize, Ordering},
			Arc,
		},
		time::{Duration, SystemTime},
	};

	static NEXT_DB: AtomicUsize = AtomicUsize::new(0);

	fn setup() {
		let _ = env_logger::builder()
			.is_test(true)
			.format_timestamp_nanos()
			.filter_level(log::LevelFilter::Debug)
			.try_init();
	}

	async fn get_client() -> Result<Arc<RedisRatelimiter>> {
		let db = NEXT_DB.fetch_add(1, Ordering::Relaxed);
		dbg!(db);

		let client = Client::open("redis://localhost:6379")?;
		let mut conn = client.get_async_connection().await?;
		let _: redis::Value = redis::cmd("SELECT").arg(db).query_async(&mut conn).await?;
		let _: redis::Value = redis::cmd("FLUSHDB").query_async(&mut conn).await?;

		let pubsub = client.get_async_connection().await?;
		let _: redis::Value = redis::cmd("SELECT").arg(db).query_async(&mut conn).await?;
		let pubsub = pubsub.into_pubsub();

		Ok(Arc::new(
			RedisRatelimiter::new_from_connections(conn, pubsub).await?,
		))
	}

	async fn claim_timeout(
		client: Arc<RedisRatelimiter>,
		bucket: &str,
		min_millis: u64,
		max_millis: u64,
	) -> Result<()> {
		let min = Duration::from_millis(min_millis);
		let max = Duration::from_millis(max_millis);
		let start = SystemTime::now();
		tokio::time::timeout(max, client.claim(bucket.to_string())).await??;

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

	#[tokio::test]
	async fn claim_release() -> Result<()> {
		setup();

		let client = get_client().await?;
		claim_timeout(client.clone(), "foo", 0, 50).await?;

		let released = Arc::new(AtomicBool::new(false));
		tokio::try_join!(
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

	#[tokio::test]
	async fn claim_timeout_release() -> Result<()> {
		setup();

		let client = get_client().await?;
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

	#[tokio::test]
	async fn claim_3x() -> Result<()> {
		setup();

		let client = get_client().await?;
		tokio::try_join!(
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

	#[tokio::test]
	async fn claim_limit_release() -> Result<()> {
		setup();

		let client = get_client().await?;
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
		tokio::try_join!(
			async {
				claim_timeout(client.clone(), "foo", 5000, 5050).await?;
				match released.load(Ordering::Relaxed) {
					true => Ok(()),
					false => Err(anyhow::anyhow!("claimed before release")),
				}
			},
			async {
				tokio::time::delay_for(Duration::from_secs(5)).await;
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

	#[tokio::test]
	async fn claim_limit_timeout() -> Result<()> {
		setup();

		let client = get_client().await?;
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

	#[tokio::test]
	async fn claim_limit_release_timeout() -> Result<()> {
		setup();

		let client = get_client().await?;
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

		tokio::time::delay_for(Duration::from_secs(1)).await;
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
