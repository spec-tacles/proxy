use super::{FutureResult, RatelimitInfo, Ratelimiter};
use anyhow::anyhow;
use log::debug;
use std::{
	collections::HashMap,
	mem::drop,
	sync::{
		atomic::{AtomicUsize, Ordering},
		Arc,
	},
};
use tokio::{
	select, spawn,
	sync::{
		mpsc::{self, Sender},
		Mutex, RwLock, Semaphore,
	},
	time::{delay_for, delay_until, Duration, Instant},
};

#[derive(Debug)]
struct Bucket {
	ready: Semaphore,
	new_timeout: Mutex<Option<Sender<Instant>>>,
	size: AtomicUsize,
}

impl Default for Bucket {
	fn default() -> Self {
		Self {
			ready: Semaphore::new(1),
			new_timeout: Default::default(),
			size: AtomicUsize::new(1),
		}
	}
}

#[derive(Debug, Default)]
pub struct LocalRatelimiter {
	buckets: RwLock<HashMap<String, Arc<Bucket>>>,
}

impl Ratelimiter for LocalRatelimiter {
	fn claim(self: Arc<Self>, bucket_name: String) -> FutureResult<()> {
		let this = Arc::clone(&self);
		Box::pin(async move {
			let mut claim = this.buckets.write().await;
			let bucket = Arc::clone(claim.entry(bucket_name.clone()).or_default());
			drop(claim);

			bucket.ready.acquire().await.forget();

			debug!("Acquired lock for \"{}\"", &bucket_name);
			Ok(())
		})
	}

	fn release(self: Arc<Self>, bucket_name: String, info: RatelimitInfo) -> FutureResult<()> {
		let this = Arc::clone(&self);
		let now = Instant::now();

		Box::pin(async move {
			debug!("Releasing \"{}\"", &bucket_name);

			let bucket = Arc::clone(
				this.buckets
					.read()
					.await
					.get(&bucket_name)
					.ok_or(anyhow!("Attempted to release before claim"))?,
			);

			let mut maybe_sender = bucket.new_timeout.lock().await;

			if let None = &*maybe_sender {
				debug!("No timeout: releasing \"{}\" immediately", &bucket_name);
				bucket.ready.add_permits(1);
			}

			if let Some(resets_in) = info.resets_in {
				let duration = Duration::from_millis(resets_in);

				debug!(
					"Received timeout of {:?} for \"{}\"",
					duration, &bucket_name
				);

				match &mut *maybe_sender {
					Some(sender) => {
						debug!("Resetting expiration for \"{}\"", &bucket_name);
						sender.send(now + duration).await?;
					}
					None => {
						debug!("Creating new expiration for \"{}\"", &bucket_name);
						let mut delay = delay_for(duration);
						let (sender, mut receiver) = mpsc::channel(1);
						let timeout_bucket = Arc::clone(&bucket);
						let bucket_name = bucket_name.clone();
						spawn(async move {
							loop {
								select! {
									Some(new_instant) = receiver.recv() => {
										debug!("Updating timeout for \"{}\" to {:?}", &bucket_name, new_instant);
										delay = delay_until(new_instant);
									},
									_ = delay => {
										debug!("Releasing \"{}\" after timeout", &bucket_name);
										let size = timeout_bucket.size.load(Ordering::SeqCst);
										timeout_bucket.ready.add_permits(size);
										*timeout_bucket.new_timeout.lock().await = None;
										break;
									}
								}
							}
						});
						*maybe_sender = Some(sender);
					}
				}
			}

			if let Some(size) = info.limit {
				let old_size = bucket.size.swap(size, Ordering::SeqCst);
				let diff = size - old_size;
				debug!(
					"New bucket size for \"{}\": {} (changing permits by {})",
					&bucket_name, size, diff
				);
				bucket.ready.add_permits(diff);
			}

			Ok(())
		})
	}
}

#[cfg(test)]
mod test {
	use super::{super::test, LocalRatelimiter};
	use anyhow::Result;
	use std::sync::Arc;

	fn get_client() -> Arc<LocalRatelimiter> {
		Default::default()
	}

	#[tokio::test]
	async fn claim_release() -> Result<()> {
		test::setup();
		test::claim_release(get_client()).await
	}

	#[tokio::test]
	async fn claim_timeout_release() -> Result<()> {
		test::setup();
		test::claim_timeout_release(get_client()).await
	}

	#[tokio::test]
	async fn claim_3x() -> Result<()> {
		test::setup();
		test::claim_3x(get_client()).await
	}

	#[tokio::test]
	async fn claim_limit_release() -> Result<()> {
		test::setup();
		test::claim_limit_release(get_client()).await
	}

	#[tokio::test]
	async fn claim_limit_timeout() -> Result<()> {
		test::setup();
		test::claim_limit_timeout(get_client()).await
	}

	#[tokio::test]
	async fn claim_limit_release_timeout() -> Result<()> {
		test::setup();
		test::claim_limit_release_timeout(get_client()).await
	}
}
