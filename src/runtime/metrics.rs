use std::{net::SocketAddr, time::Instant};

use lazy_static::lazy_static;
use prometheus::{Encoder, HistogramVec, TextEncoder};
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

pub struct LatencyTracker<'vec, 'labels> {
	vec: &'vec HistogramVec,
	labels: &'labels [&'labels str],
	start: Instant,
}

impl<'vec, 'labels> LatencyTracker<'vec, 'labels> {
	pub fn new(vec: &'vec HistogramVec, labels: &'labels [&'labels str]) -> Self {
		Self {
			vec,
			labels,
			start: Instant::now(),
		}
	}
}

impl<'vec, 'labels> Drop for LatencyTracker<'vec, 'labels> {
	fn drop(&mut self) {
		let latency = Instant::now().duration_since(self.start);
		self.vec
			.get_metric_with_label_values(self.labels)
			.unwrap()
			.observe(latency.as_secs_f64());
	}
}
