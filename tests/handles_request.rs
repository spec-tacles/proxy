use anyhow::Result;
use bytes::Bytes;
use futures::TryStreamExt;
use mockito::mock;
use rustacles_brokers::common::Rpc;
use rustacles_brokers::redis::redust::pool::{Manager, Pool};
use rustacles_brokers::redis::RedisBroker;
use spectacles_proxy::ratelimiter::local::LocalRatelimiter;
use spectacles_proxy::{
	models::{
		RequestResponse, RequestResponseBody, ResponseStatus, SerializableHttpRequest,
		SerializableHttpResponse,
	},
	runtime::{Client, Config},
};
use std::sync::Arc;
use test_log::test;
use tokio::{
	spawn,
	time::{timeout, Duration},
};

#[test(tokio::test)]
async fn handles_request() -> Result<()> {
	let config = dbg!(Config::default().with_env());
	let manager = Manager::new(config.redis.url.clone());
	let pool = Pool::builder(manager)
		.max_size(config.redis.pool_size)
		.build()
		.expect("pool should be built");
	let broker = RedisBroker::new(config.broker.group.clone(), pool.clone());

	let rpc_broker = RedisBroker::new(config.broker.group, pool);

	let ratelimiter = LocalRatelimiter::default();
	let mock_addr = mockito::server_address();

	let client = Client {
		api_base: mock_addr.to_string(),
		api_scheme: uriparse::Scheme::HTTP,
		api_version: 6,
		http: reqwest::Client::new(),
		ratelimiter: Arc::new(ratelimiter),
		timeout: None,
	};

	let mock = mock("GET", "/api/v6/foo/bar")
		.with_body(rmp_serde::to_vec(&["hello world"])?)
		.create();

	let events = vec![Bytes::from(config.broker.event.clone())];
	broker.ensure_events(events.iter()).await?;
	spawn(async move {
		let mut consumer = broker.consume(events);
		while let Some(message) = consumer.try_next().await.expect("Next message") {
			client
				.handle_message(message)
				.await
				.expect("Unable to handle message");
		}
	});

	let payload = SerializableHttpRequest {
		method: "GET".into(),
		path: "/foo/bar".into(),
		query: None,
		body: None,
		headers: Default::default(),
		timeout: None,
	};

	let rpc = timeout(
		Duration::from_secs(5),
		rpc_broker.call(config.broker.event.as_str(), &payload, None),
	)
	.await??;

	let response = rpc
		.response::<RequestResponse<SerializableHttpResponse>>()
		.await?
		.unwrap();
	mock.assert();

	assert_eq!(response.status, ResponseStatus::Success);
	assert_eq!(
		response.body,
		RequestResponseBody::Ok(SerializableHttpResponse {
			status: 200,
			headers: vec![
				("connection".to_string(), "close".to_string()),
				("content-length".to_string(), "13".to_string())
			]
			.into_iter()
			.collect(),
			url: format!("http://{}/api/v6/foo/bar", mock_addr),
			body: rmp_serde::to_vec(&["hello world"])?.into(),
		})
	);

	Ok(())
}
