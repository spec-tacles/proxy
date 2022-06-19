use lazy_static::lazy_static;
use prometheus::{register_histogram_vec, register_int_counter_vec, HistogramVec, IntCounterVec};

lazy_static! {
	pub static ref REQUESTS_TOTAL: IntCounterVec = register_int_counter_vec!(
		"proxy_requests_total",
		"Number of HTTP requests made",
		&["method", "path"]
	)
	.unwrap();
	pub static ref RESPONSES_TOTAL: IntCounterVec = register_int_counter_vec!(
		"proxy_responses_total",
		"Number of HTTP responses received",
		&["method", "path", "status"]
	)
	.unwrap();
	pub static ref REQUEST_LATENCY: HistogramVec = register_histogram_vec!(
		"proxy_request_latency",
		"Latency of HTTP requests (in seconds)",
		&["method", "path"]
	)
	.unwrap();
	pub static ref RATELIMIT_LATENCY: HistogramVec = register_histogram_vec!(
		"proxy_ratelimit_latency",
		"Latency of ratelimit checking, including wait time for any ratelimited requests.",
		&["method", "path"]
	)
	.unwrap();
}
