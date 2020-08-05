use crate::{
	config::Config, Client
};
use anyhow::Result;
use mockito::mock;
use rustacles_brokers::amqp::AmqpBroker;
use serde_json::{from_slice, json, to_vec};
use spectacles_proxy::{
	models::*,
	ratelimiter::{local::LocalRatelimiter, reqwest}
};
use std::sync::Arc;
use tokio::{
	spawn,
	time::{delay_for, timeout, Duration},
};

#[tokio::test]
async fn handles_request() -> Result<()> {
	let config = Config::default().with_env();
	let broker: AmqpBroker = loop {
		let broker_res = AmqpBroker::new(
			&config.amqp.url,
			config.amqp.group.clone(),
			config.amqp.subgroup.clone(),
		)
		.await;

		if let Ok(b) = broker_res {
			break b;
		}

		delay_for(Duration::from_secs(5)).await;
	};

	let rpc_broker = AmqpBroker::new(&config.amqp.url, config.amqp.group, config.amqp.subgroup)
		.await?
		.with_rpc()
		.await?;

	let mut consumer = broker.consume(&config.amqp.event).await?;

	let ratelimiter = LocalRatelimiter::default();

	let client = Client {
		api_base: mockito::SERVER_ADDRESS,
		api_scheme: uriparse::Scheme::HTTP,
		api_version: 6,
		broker: Arc::new(broker),
		http: Arc::new(reqwest::Client::new()),
		ratelimiter: Arc::new(ratelimiter),
		timeout: None,
	};

	let mock = mock("GET", "/api/v6/foo/bar")
		.with_body("[\"hello world\"]")
		.create();
	let mock_addr = mockito::server_address();

	spawn(async move {
		while let Some(message) = consumer.recv().await {
			client.handle_request(&message).await;
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

	let response = timeout(
		Duration::from_secs(5),
		rpc_broker.call(
			&config.amqp.event,
			to_vec(&payload).unwrap(),
			Default::default(),
		),
	)
	.await??;
	mock.assert();

	let response: RequestResponse<SerializableHttpResponse> = from_slice(&response.data)?;

	assert_eq!(response.status, ResponseStatus::Success);
	assert_eq!(
		response.body,
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
