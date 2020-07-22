use super::{FutureResult, RatelimitInfo, Ratelimiter};
use anyhow::anyhow;
use std::{
	collections::HashMap,
	sync::{
		atomic::{AtomicUsize, Ordering},
		Arc,
	},
	time::SystemTime,
};
use tokio::sync::{Notify, RwLock};

#[derive(Debug)]
struct Bucket {
	size: Option<usize>,
	remaining: AtomicUsize,
	expires_at: Option<SystemTime>,
	ready_notifier: Option<Notify>,
}

impl Default for Bucket {
	fn default() -> Self {
		Self {
			size: None,
			remaining: AtomicUsize::new(1),
			expires_at: None,
			ready_notifier: None,
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
			loop {
				let bucket = Arc::clone(
					this.buckets
						.write()
						.await
						.entry(bucket.clone())
						.or_default(),
				);

				let updated =
					bucket
						.remaining
						.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |x| x.checked_sub(1));

				if updated.is_ok() {
					break;
				}

				if let Some(expiration) = bucket.expires_at {
					if let Err(until) = expiration.elapsed() {
						tokio::time::delay_for(until.duration()).await;
					}

					continue;
				} else if let Some(ready_notifier) = bucket.ready_notifier.as_ref() {
					ready_notifier.notified().await;
					continue;
				} else {
					return Err(anyhow!("Unable to get waiting duration"));
				}
			}

			Ok(())
		})
	}

	fn release(self: Arc<Self>, bucket: String, info: RatelimitInfo) -> FutureResult<()> {
		let this = Arc::clone(&self);

		Box::pin(async move {
			let bucket = this.buckets
				.read()
				.await
				.get(&bucket)
				.ok_or(anyhow!("Attempted to release before claim"))?;

			Ok(())
		})
	}
}
