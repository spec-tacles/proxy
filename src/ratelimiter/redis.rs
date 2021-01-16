use super::{FutureResult, RatelimitInfo, Ratelimiter};
use anyhow::Result;
use log::debug;
use redis::Script;
use std::{sync::Arc, time::Duration};
use tokio::sync::{broadcast, Mutex};
use tokio_stream::StreamExt;

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
					tokio::time::sleep(Duration::from_millis(expiration as u64)).await;
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
	use super::{super::test, RedisRatelimiter};
	use anyhow::Result;
	use redis::Client;
	use std::sync::{
		atomic::{AtomicUsize, Ordering},
		Arc,
	};

	static NEXT_DB: AtomicUsize = AtomicUsize::new(0);

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

	#[tokio::test]
	async fn claim_release() -> Result<()> {
		test::setup();
		let client = get_client().await?;
		test::claim_release(client).await
	}

	#[tokio::test]
	async fn claim_timeout_release() -> Result<()> {
		test::setup();
		let client = get_client().await?;
		test::claim_timeout_release(client).await
	}

	#[tokio::test]
	async fn claim_3x() -> Result<()> {
		test::setup();
		let client = get_client().await?;
		test::claim_3x(client).await
	}

	#[tokio::test]
	async fn claim_limit_release() -> Result<()> {
		test::setup();
		let client = get_client().await?;
		test::claim_limit_release(client).await
	}

	#[tokio::test]
	async fn claim_limit_timeout() -> Result<()> {
		test::setup();
		let client = get_client().await?;
		test::claim_limit_timeout(client).await
	}

	#[tokio::test]
	async fn claim_limit_release_timeout() -> Result<()> {
		test::setup();
		let client = get_client().await?;
		test::claim_limit_release_timeout(client).await
	}
}
