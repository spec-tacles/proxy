use super::{FutureResult, RatelimitInfo, Ratelimiter};
use anyhow::Result;
use lazy_static::lazy_static;
use log::debug;
use redis::{
	aio::{MultiplexedConnection, PubSub},
	Script,
};
use std::time::Duration;
use tokio::sync::broadcast;
use tokio_stream::StreamExt;

static NOTIFY_KEY: &'static str = "rest_ready";

lazy_static! {
	static ref CLAIM_SCRIPT: Script = Script::new(include_str!("./scripts/claim.lua"));
	static ref RELEASE_SCRIPT: Script = Script::new(include_str!("./scripts/release.lua"));
}

pub struct RedisRatelimiter {
	redis: MultiplexedConnection,
	ready_publisher: broadcast::Sender<String>,
}

impl RedisRatelimiter {
	pub async fn new(redis: &redis::Client) -> Result<Self> {
		let pubsub = redis.get_async_connection().await?.into_pubsub();
		let main = redis.get_multiplexed_tokio_connection().await?;
		Self::new_from_connections(main, pubsub).await
	}

	pub async fn new_from_connections(
		main: MultiplexedConnection,
		mut pubsub: PubSub,
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
			redis: main,
			ready_publisher: sender,
		})
	}
}

impl Ratelimiter for RedisRatelimiter {
	fn claim(&self, bucket: String) -> FutureResult<()> {
		let mut conn = self.redis.clone();
		let mut rcv = self.ready_publisher.subscribe();
		Box::pin(async move {
			'outer: loop {
				let expiration: isize = CLAIM_SCRIPT
					.key(&bucket)
					.key(bucket.to_string() + "_size")
					.invoke_async(&mut conn)
					.await?;

				debug!("Received expiration of {}ms for \"{}\"", expiration, bucket);

				if expiration.is_positive() {
					tokio::time::sleep(Duration::from_millis(expiration as u64)).await;
					continue;
				}

				if expiration == 0 {
					break;
				}

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

	fn release(&self, bucket: String, info: RatelimitInfo) -> FutureResult<()> {
		let mut conn = self.redis.clone();
		Box::pin(async move {
			RELEASE_SCRIPT
				.key(&bucket)
				.key(bucket.to_string() + "_size")
				.key(NOTIFY_KEY)
				.arg(info.limit.unwrap_or(0))
				.arg(info.resets_in.unwrap_or(0))
				.invoke_async(&mut conn)
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
		let mut conn = client.get_multiplexed_tokio_connection().await?;
		let _: redis::Value = redis::cmd("SELECT").arg(db).query_async(&mut conn).await?;
		let _: redis::Value = redis::cmd("FLUSHDB").query_async(&mut conn).await?;

		let pubsub = client.get_async_connection().await?;
		let _: redis::Value = redis::cmd("SELECT").arg(db).query_async(&mut conn).await?;
		let pubsub = pubsub.into_pubsub();

		Ok(Arc::new(
			RedisRatelimiter::new_from_connections(client, pubsub).await?,
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
