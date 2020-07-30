use crate::{config::AmqpConfig, Client, SerializableHttpRequest, SerializableHttpResponse};
use anyhow::Result;
use mockito::mock;
use rustacles_brokers::amqp::AmqpBroker;
use serde_json::{from_slice, json, to_vec};
use spectacles_proxy::ratelimiter::{local::LocalRatelimiter, reqwest};
use std::sync::Arc;
use tokio::{
	spawn,
	time::{timeout, Duration},
};

#[tokio::test]
async fn handles_request() -> Result<()> {
	let amqp_config = AmqpConfig::default();
	let broker = AmqpBroker::new(
		&amqp_config.url,
		amqp_config.group.clone(),
		amqp_config.subgroup.clone(),
	)
	.await?;

	let rpc_broker = AmqpBroker::new(&amqp_config.url, amqp_config.group, amqp_config.subgroup)
		.await?
		.with_rpc()
		.await?;

	let mut consumer = broker.consume(&amqp_config.event).await?;

	let ratelimiter = LocalRatelimiter::default();

	let client = Client {
		api_base: mockito::SERVER_ADDRESS,
		api_scheme: uriparse::Scheme::HTTP,
		api_version: 6,
		broker,
		http: Arc::new(reqwest::Client::new()),
		ratelimiter: Arc::new(ratelimiter),
	};

	let mock = mock("GET", "/api/v6/foo/bar")
		.with_body("[\"hello world\"]")
		.create();
	let mock_addr = mockito::server_address();

	spawn(async move {
		while let Some(message) = consumer.recv().await {
			client
				.handle_request(message)
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
	};

	let response = timeout(
		Duration::from_secs(5),
		rpc_broker.call(
			&amqp_config.event,
			to_vec(&payload).unwrap(),
			Default::default(),
		),
	)
	.await??;
	mock.assert();

	let body: SerializableHttpResponse = from_slice(&response.data)?;

	assert_eq!(
		body,
		SerializableHttpResponse {
			status: 200,
			headers: vec![
				("connection".to_string(), "close".to_string()),
				("content-length".to_string(), "15".to_string())
			]
			.into_iter()
			.collect(),
			url: format!("http://{}/api/v6/foo/bar", mock_addr),
			body: json!(["hello world"]),
		}
	);

	Ok(())
}
