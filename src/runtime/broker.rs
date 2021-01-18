use std::{collections::HashMap, future::Future, sync::Arc, time::Duration};

use log::{error, trace, warn};
use rustacles_brokers::amqp::{AmqpBroker, Message};
use tokio::{
	select, spawn,
	sync::{mpsc::UnboundedReceiver, Mutex, Notify},
	time::sleep,
};

use super::Config;

type Cancellations = Arc<Mutex<HashMap<String, Arc<Notify>>>>;

pub struct Broker {
	consumer: UnboundedReceiver<Message>,
	cancellations: Cancellations,
}

impl Broker {
	pub async fn from_config(config: &Config) -> Self {
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

		let consumer = broker
			.consume(&config.amqp.event)
			.await
			.expect("Unable to setup message consumption");

		let mut cancellation_consumer = broker
			.consume(&config.amqp.cancellation_event)
			.await
			.expect("Unable to setup cancellation message consumption");

		let cancellations = Cancellations::default();
		let consume_cancellations = Arc::clone(&cancellations);
		spawn(async move {
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

		Self {
			consumer,
			cancellations,
		}
	}

	pub async fn consume_messages<T, F: Future<Output = T> + Send + 'static>(
		mut self,
		handler: impl Fn(Message) -> F + Send + Sync + Clone + 'static,
	) {
		while let Some(message) = self.consumer.recv().await {
			let cancellations = Arc::clone(&self.cancellations);
			let handler = handler.clone();

			trace!("Received message");
			spawn(async move {
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

				let fut = handler(message);

				select! {
					_ = fut => {},
					_ = cancellation.notified() => {
						return;
					}
				}

				if let Some(correlation_id) = &maybe_correlation_id {
					cancellations.lock().await.remove(correlation_id);
				}
			});
		}
	}
}
