use std::net::SocketAddr;

use lazy_static::lazy_static;
use prometheus::{Encoder, TextEncoder};
use warp::Filter;

lazy_static! {
	static ref TEXT_ENCODER: TextEncoder = TextEncoder::new();
}

pub async fn start_server(path: String, addr: impl Into<SocketAddr>) {
	let route = warp::path(path).and(warp::get()).map(|| {
		let metrics = prometheus::gather();
		let mut out = vec![];
		TEXT_ENCODER
			.encode(&metrics, &mut out)
			.expect("unable to encode response");
		String::from_utf8(out).expect("valid UTF-8 metrics")
	});

	warp::serve(route).run(addr).await;
}
