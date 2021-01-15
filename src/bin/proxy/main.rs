#![feature(iterator_fold_self, maybe_uninit_ref)]

use log::{error, info, trace, warn};
use rustacles_brokers::amqp::AmqpBroker;
use spectacles_proxy::{
	models::*,
	ratelimiter::{
		redis::{redis, RedisRatelimiter},
		reqwest,
	},
};
use std::{collections::HashMap, sync::Arc};
use tokio::{
	select,
	sync::{Mutex, Notify},
	time::{sleep, Duration},
};
use uriparse::Scheme;

mod client;
mod config;
#[cfg(test)]
mod test;

pub use client::*;
pub use config::*;

#[tokio::main]
async fn main() {
	env_logger::init();

	let config = Config::from_toml_file("proxy.toml")
		.unwrap_or_default()
		.with_env();

	let redis_client = redis::Client::open(config.redis.url).expect("Unable to connect to Redis");
	let broker = Arc::new(loop {
		match AmqpBroker::new(
			&config.amqp.url,
			config.amqp.group.clone(),
			config.amqp.subgroup.clone(),
		)
		.await
		{
			Ok(b) => break b,
			Err(e) => error!("Error connecting to AMQP; retrying in 5s: {}", e),
		}

		sleep(Duration::from_secs(5)).await;
	});

	let mut consumer = broker
		.consume(&config.amqp.event)
		.await
		.expect("Unable to setup message consumption");

	let mut cancellation_consumer = broker
		.consume(&config.amqp.cancellation_event)
		.await
		.expect("Unable to setup cancellation message consumption");

	let ratelimiter = loop {
		match RedisRatelimiter::new(&redis_client).await {
			Ok(r) => break r,
			Err(e) => error!("Error setting up Redis ratelimiter; retrying in 5s: {}", e),
		}

		sleep(Duration::from_secs(5)).await;
	};

	let cancellations: Arc<Mutex<HashMap<String, Arc<Notify>>>> = Default::default();
	let consume_cancellations = Arc::clone(&cancellations);
	tokio::spawn(async move {
		while let Some(message) = cancellation_consumer.recv().await {
			if let Ok(id) = String::from_utf8(message.data) {
				trace!("Received cancellation for request \"{}\"", &id);
				consume_cancellations
					.lock()
					.await
					.remove(&id)
					.map(|n| n.notify_waiters());
			} else {
				warn!("Received invalid UTF-8 cancellation request data");
			}
		}
	});

	let client = Arc::new(Client {
		http: reqwest::Client::new(),
		ratelimiter: Arc::new(ratelimiter),
		api_base: "discord.com",
		api_scheme: Scheme::HTTPS,
		api_version: config.discord.api_version,
		timeout: config.timeout.map(|d| d.into()),
	});

	info!("Beginning normal message consumption");
	while let Some(message) = consumer.recv().await {
		let client = Arc::clone(&client);
		let cancellations = Arc::clone(&cancellations);

		trace!("Received message");
		tokio::spawn(async move {
			let cancellation = Arc::new(Notify::new());
			let maybe_correlation_id = message
				.properties
				.correlation_id()
				.as_ref()
				.map(|id| id.to_string());

			if let Some(correlation_id) = &maybe_correlation_id {
				cancellations
					.lock()
					.await
					.insert(correlation_id.clone(), Arc::clone(&cancellation));
			}

			select! {
				_ = cancellation.notified() => {
					trace!("Request cancelled");
					// cancellation notifier is removed during notification, so we can exit here to avoid an extra lock
					return;
				},
				result = client.handle_message(&message) => match result {
					Ok(_) => trace!("Request completed"),
					Err(e) => warn!("Request failed: {}", e),
				},
			};

			if let Some(correlation_id) = &maybe_correlation_id {
				cancellations.lock().await.remove(correlation_id);
			}
		});
	}
}
