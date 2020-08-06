use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use serde_repr::*;
use spectacles_proxy::ratelimiter::reqwest;
use std::collections::HashMap;
use tokio::time::Duration;

#[derive(Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct SerializableHttpRequest {
	pub method: String,
	pub path: String,
	pub query: Option<HashMap<String, String>>,
	pub body: Option<Value>,
	#[serde(default)]
	pub headers: HashMap<String, String>,
	pub timeout: Option<Duration>,
}

#[derive(Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct SerializableHttpResponse {
	pub status: u16,
	pub headers: HashMap<String, String>,
	pub url: String,
	pub body: Value,
}

#[repr(u8)]
#[derive(Debug, Serialize_repr, Deserialize_repr, Eq, PartialEq)]
pub enum ResponseStatus {
	Success,
	Unknown,
	InvalidRequestFormat,
	InvalidPath,
	InvalidQuery,
	InvalidMethod,
	InvalidHeaders,
	RequestFailure,
	RequestTimeout,
}

impl From<&(dyn std::error::Error + 'static)> for ResponseStatus {
	fn from(e: &(dyn std::error::Error + 'static)) -> Self {
		if e.is::<serde_json::Error>() {
			ResponseStatus::InvalidRequestFormat
		} else if e.is::<uriparse::PathError>() {
			ResponseStatus::InvalidPath
		} else if e.is::<uriparse::QueryError>() {
			ResponseStatus::InvalidQuery
		} else if e.is::<http::method::InvalidMethod>() {
			ResponseStatus::InvalidMethod
		} else if e.is::<http::Error>() {
			ResponseStatus::InvalidHeaders
		} else if e.is::<reqwest::Error>() {
			ResponseStatus::RequestFailure
		} else {
			ResponseStatus::Unknown
		}
	}
}

#[derive(Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct RequestResponse<T> {
	pub status: ResponseStatus,
	pub body: T,
}

impl<T> From<Result<T>> for RequestResponse<Value>
where
	T: Serialize,
{
	fn from(res: Result<T>) -> Self {
		match res {
			Err(e) => {
				let e_ref: &(dyn std::error::Error) = e.as_ref();
				Self {
					status: e_ref.into(),
					body: serde_json::to_value(e.to_string())
						.expect("Unable to serialize response error"),
				}
			}
			Ok(t) => Self {
				status: ResponseStatus::Success,
				body: serde_json::to_value(t).expect("Unable to serialize response data"),
			},
		}
	}
}
