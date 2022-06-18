use super::{RatelimitInfo, Ratelimiter};
use anyhow::Result;
use async_trait::async_trait;
use futures::TryStreamExt;
use lazy_static::lazy_static;
use redust::{model::pubsub, pool::Pool, resp::from_data, script::Script};
use std::{fmt::Debug, str::from_utf8, time::Duration};
use tokio::{net::ToSocketAddrs, time::sleep};
use tracing::{debug, instrument};

static NOTIFY_KEY: &'static str = "rest_ready";

lazy_static! {
	static ref CLAIM_SCRIPT: Script<2> = Script::new(include_bytes!("./scripts/claim.lua"));
	static ref RELEASE_SCRIPT: Script<3> = Script::new(include_bytes!("./scripts/release.lua"));
}

#[derive(Clone, Debug)]
pub struct RedisRatelimiter<A>
where
	A: ToSocketAddrs + Clone + Send + Sync + Debug,
{
	redis: Pool<A>,
}

impl<A> RedisRatelimiter<A>
where
	A: ToSocketAddrs + Clone + Send + Sync + Debug + 'static,
{
	pub fn new(pool: Pool<A>) -> Self {
		Self { redis: pool }
	}
}

#[async_trait]
impl<A> Ratelimiter for RedisRatelimiter<A>
where
	A: ToSocketAddrs + Clone + Send + Sync + Debug,
{
	#[instrument(level = "debug")]
	async fn claim(&self, bucket: String) -> Result<()> {
		loop {
			let mut conn = self.redis.get().await?;
			let expiration = CLAIM_SCRIPT
				.exec(&mut conn)
				.keys([&bucket, &(bucket.to_string() + "_size")])
				.invoke()
				.await?;
			let expiration = from_data::<i64>(expiration)?;

			debug!("Received expiration of {}ms for \"{}\"", expiration, bucket);

			if expiration.is_positive() {
				sleep(Duration::from_millis(expiration as u64)).await;
				continue;
			}

			if expiration == 0 {
				break;
			}

			conn.cmd(["SUBSCRIBE", NOTIFY_KEY]).await?;
			while let Some(data) = conn.try_next().await? {
				let res = from_data::<pubsub::Response>(data)?;
				match res {
					pubsub::Response::Message(msg) if from_utf8(&msg.data) == Ok(&bucket) => {
						break;
					}
					_ => {}
				}
			}
			conn.cmd(["UNSUBSCRIBE", NOTIFY_KEY]).await?;
		}

		Ok(())
	}

	#[instrument(level = "debug")]
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
	use std::sync::{
		atomic::{AtomicUsize, Ordering},
		Arc,
	};

	use anyhow::Result;
	use redust::pool::{deadpool::managed::HookFuture, Hook, Manager, Pool};
	use test_log::test;

	use super::{super::test, RedisRatelimiter};

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

		Ok(Arc::new(RedisRatelimiter::new(pool)))
	}

	#[test(tokio::test)]
	async fn claim_release() -> Result<()> {
		let client = get_client().await?;
		test::claim_release(client).await
	}

	#[test(tokio::test)]
	async fn claim_timeout_release() -> Result<()> {
		let client = get_client().await?;
		test::claim_timeout_release(client).await
	}

	#[test(tokio::test)]
	async fn claim_3x() -> Result<()> {
		let client = get_client().await?;
		test::claim_3x(client).await
	}

	#[test(tokio::test)]
	async fn claim_limit_release() -> Result<()> {
		let client = get_client().await?;
		test::claim_limit_release(client).await
	}

	#[test(tokio::test)]
	async fn claim_limit_timeout() -> Result<()> {
		let client = get_client().await?;
		test::claim_limit_timeout(client).await
	}

	#[test(tokio::test)]
	async fn claim_limit_release_timeout() -> Result<()> {
		let client = get_client().await?;
		test::claim_limit_release_timeout(client).await
	}
}
