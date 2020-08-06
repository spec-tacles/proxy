#![feature(iterator_fold_self)]

use rustacles_brokers::amqp::AmqpBroker;
use spectacles_proxy::{
	models::*,
	ratelimiter::{
		redis::{redis, RedisRatelimiter},
		reqwest,
	}
};
use std::{mem::drop, sync::Arc};
use tokio::{
	select,
	signal::ctrl_c,
	sync::mpsc,
	time::{delay_for, Duration},
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
			Err(e) => eprintln!("Error connecting to AMQP; retrying in 5s: {}", e),
		}

		delay_for(Duration::from_secs(5)).await;
	});

	let mut consumer = broker
		.consume(&config.amqp.event)
		.await
		.expect("Unable to setup message consumption");

	let ratelimiter = loop {
		match RedisRatelimiter::new(&redis_client).await {
			Ok(r) => break r,
			Err(e) => eprintln!("Error setting up Redis ratelimiter; retrying in 5s: {}", e),
		}

		delay_for(Duration::from_secs(5)).await;
	};

	let client = Arc::new(Client {
		http: Arc::new(reqwest::Client::new()),
		broker: Arc::clone(&broker),
		ratelimiter: Arc::new(ratelimiter),
		api_base: "discord.com",
		api_scheme: Scheme::HTTPS,
		api_version: 6,
		timeout: None,
	});

	let (republish_send, mut republish_recv) = mpsc::unbounded_channel::<Vec<u8>>();

	println!("Beginning normal message consumption");
	while let Some(message) = select! {
		_ = ctrl_c() => None,
		m = consumer.recv() => m,
	} {
		let client = Arc::clone(&client);
		let republish_send = republish_send.clone();
		tokio::spawn(async move {
			select! {
				_ = ctrl_c() => {
					republish_send.send(message.data).expect("Unable to republish message on exit");
				},
				_ = client.handle_request(&message) => {}
			};
		});
	}

	drop(client);
	drop(consumer);
	drop(republish_send);

	println!("Flushing incomplete messages back to AMQP before exiting");
	while let Some(data) = republish_recv.recv().await {
		broker
			.publish("REQUEST", data, Default::default())
			.await
			.expect("Unable to republish message to AMQP");
	}
}
