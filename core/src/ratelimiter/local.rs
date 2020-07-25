use super::{FutureResult, RatelimitInfo, Ratelimiter};
use anyhow::anyhow;
use std::{collections::HashMap, sync::Arc};
use tokio::{
	sync::{
		mpsc::{self, Sender},
		Mutex, OwnedSemaphorePermit, RwLock, Semaphore,
	},
	time::{delay_for, delay_until, Duration, Instant},
};

#[derive(Debug)]
struct Bucket {
	ready: Arc<Semaphore>,
	permits: Mutex<Vec<OwnedSemaphorePermit>>,
	new_timeout: Mutex<Option<Sender<Instant>>>,
	size: usize,
}

impl Default for Bucket {
	fn default() -> Self {
		Self {
			ready: Arc::new(Semaphore::new(1)),
			permits: Default::default(),
			new_timeout: Default::default(),
			size: 1,
		}
	}
}

#[derive(Debug)]
pub struct LocalRatelimiter {
	buckets: RwLock<HashMap<String, Arc<Bucket>>>,
}

impl Ratelimiter for LocalRatelimiter {
	fn claim(self: Arc<Self>, bucket: String) -> FutureResult<()> {
		let this = Arc::clone(&self);
		Box::pin(async move {
			let bucket = Arc::clone(
				this.buckets
					.write()
					.await
					.entry(bucket.clone())
					.or_default(),
			);

			bucket
				.permits
				.lock()
				.await
				.push(Arc::clone(&bucket.ready).acquire_owned().await);

			Ok(())
		})
	}

	fn release(self: Arc<Self>, bucket: String, info: RatelimitInfo) -> FutureResult<()> {
		let this = Arc::clone(&self);
		let now = Instant::now();

		Box::pin(async move {
			let bucket = Arc::clone(
				this.buckets
					.read()
					.await
					.get(&bucket)
					.ok_or(anyhow!("Attempted to release before claim"))?,
			);

			if let Some(resets_in) = info.resets_in {
				let duration = Duration::from_millis(resets_in);
				let mut maybe_sender = bucket.new_timeout.lock().await;
				match &mut *maybe_sender {
					Some(sender) => {
						sender.send(now + duration).await?;
					}
					None => {
						let mut delay = delay_for(duration);
						let (sender, mut receiver) = mpsc::channel::<Instant>(1);
						let timeout_bucket = Arc::clone(&bucket);
						tokio::spawn(async move {
							loop {
								tokio::select! {
									Some(new_instant) = receiver.recv() => {
										delay = delay_until(new_instant);
									},
									_ = delay => {
										timeout_bucket.permits.lock().await.clear();
										break;
									}
								}
							}
						});
						*maybe_sender = Some(sender);
					}
				}
			}

			Ok(())
		})
	}
}

#[cfg(test)]
mod test {
	//
}
