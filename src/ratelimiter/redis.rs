use super::{RatelimitInfo, Ratelimiter};
use anyhow::Result;
use async_trait::async_trait;
use lazy_static::lazy_static;
use log::debug;
use redust::{model::pubsub, pool::Pool, resp::from_data, script::Script};
use std::{fmt::Debug, str::from_utf8, time::Duration};
use tokio::{net::ToSocketAddrs, spawn, sync::broadcast};

static NOTIFY_KEY: &'static str = "rest_ready";

lazy_static! {
	static ref CLAIM_SCRIPT: Script<2> = Script::new(include_bytes!("./scripts/claim.lua"));
	static ref RELEASE_SCRIPT: Script<3> = Script::new(include_bytes!("./scripts/release.lua"));
}

#[derive(Clone)]
pub struct RedisRatelimiter<A>
where
	A: ToSocketAddrs + Clone + Send + Sync + Debug,
{
	redis: Pool<A>,
	ready_publisher: broadcast::Sender<Vec<u8>>,
}

impl<A> RedisRatelimiter<A>
where
	A: ToSocketAddrs + Clone + Send + Sync + Debug + 'static,
{
	pub async fn new(pool: Pool<A>) -> Result<Self> {
		let (sender, _) = broadcast::channel(32);
		let mut sub_conn = pool.get().await?;

		let pubsub_sender = sender.clone();
		spawn(async move {
			sub_conn.cmd(["SUBSCRIBE", NOTIFY_KEY]).await.unwrap();
			loop {
				match from_data(sub_conn.read_cmd().await.unwrap()).unwrap() {
					pubsub::Response::Message(msg) => {
						let _ = pubsub_sender.send(msg.data.into_owned());
					}
					_ => {}
				}
			}
			// sub_conn.cmd(["UNSUBSCRIBE", NOTIFY_KEY]).await.unwrap();
		});

		Ok(Self {
			redis: pool,
			ready_publisher: sender,
		})
	}
}

#[async_trait]
impl<A> Ratelimiter for RedisRatelimiter<A>
where
	A: ToSocketAddrs + Clone + Send + Sync + Debug,
{
	async fn claim(&self, bucket: String) -> Result<()> {
		'outer: loop {
			let mut conn = self.redis.get().await?;
			let expiration = CLAIM_SCRIPT
				.exec(&mut conn)
				.keys([&bucket, &(bucket.to_string() + "_size")])
				.invoke()
				.await?;
			let expiration = from_data::<i64>(expiration)?;

			debug!("Received expiration of {}ms for \"{}\"", expiration, bucket);

			if expiration.is_positive() {
				tokio::time::sleep(Duration::from_millis(expiration as u64)).await;
				continue;
			}

			if expiration == 0 {
				break;
			}

			let mut sub = self.ready_publisher.subscribe();
			loop {
				if from_utf8(&sub.recv().await?) == Ok(&bucket) {
					continue 'outer;
				}
			}
		}

		Ok(())
	}

	async fn release(&self, bucket: String, info: RatelimitInfo) -> Result<()> {
		let mut conn = self.redis.get().await?;
		RELEASE_SCRIPT
			.exec(&mut conn)
			.keys([bucket.as_str(), &(bucket.to_string() + "_size"), NOTIFY_KEY])
			.args(&[
				info.limit.unwrap_or(0).to_string(),
				info.resets_in.unwrap_or(0).to_string(),
			])
			.invoke()
			.await?;

		Ok(())
	}
}

#[cfg(test)]
mod test {
	use super::{super::test, RedisRatelimiter};
	use anyhow::Result;
	use redust::pool::{deadpool::managed::HookFuture, Hook, Manager, Pool};
	use std::sync::{
		atomic::{AtomicUsize, Ordering},
		Arc,
	};

	static NEXT_DB: AtomicUsize = AtomicUsize::new(0);

	async fn get_client() -> Result<Arc<RedisRatelimiter<&'static str>>> {
		let db = NEXT_DB.fetch_add(1, Ordering::Relaxed);
		dbg!(db);

		let manager = Manager::new("localhost:6379");
		let pool = Pool::builder(manager)
			.post_create(Hook::async_fn(
				move |conn, _metrics| -> HookFuture<redust::Error> {
					Box::pin(async move {
						conn.cmd(["SELECT", &db.to_string()]).await.unwrap();
						conn.cmd(["FLUSHDB"]).await.unwrap();
						Ok(())
					})
				},
			))
			.build()?;

		Ok(Arc::new(RedisRatelimiter::new(pool).await?))
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
