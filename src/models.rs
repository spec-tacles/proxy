use anyhow::Result;
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use serde_repr::*;
use std::{
	collections::HashMap,
	fmt::{self, Display, Formatter},
};
use tokio::time::{error::Elapsed, Duration};

#[derive(Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct SerializableHttpRequest {
	pub method: String,
	pub path: String,
	pub query: Option<HashMap<String, String>>,
	pub body: Option<Bytes>,
	#[serde(default)]
	pub headers: HashMap<String, String>,
	pub timeout: Option<Duration>,
}

impl Display for SerializableHttpRequest {
	fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
		write!(
			f,
			"{} {} Query={:?} Headers={:?} BodyLen={:?} Timeout={:?}ms",
			self.method,
			self.path,
			self.query,
			self.headers,
			self.body.as_ref().map(|b| b.len()),
			self.timeout.map(|d| d.as_millis())
		)
	}
}

#[derive(Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct SerializableHttpResponse {
	pub status: u16,
	pub headers: HashMap<String, String>,
	pub url: String,
	pub body: Bytes,
}

impl Display for SerializableHttpResponse {
	fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
		write!(
			f,
			"{} {} Headers={:?} BodyLen={}",
			self.status,
			self.url,
			self.headers,
			self.body.len()
		)
	}
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
		if e.is::<rmp_serde::decode::Error>() {
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
		} else if e.is::<Elapsed>() {
			ResponseStatus::RequestTimeout
		} else {
			ResponseStatus::Unknown
		}
	}
}

#[derive(Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct RequestResponse<T> {
	pub status: ResponseStatus,
	pub body: RequestResponseBody<T>,
}

#[derive(Debug, Serialize, Deserialize, Eq, PartialEq)]
#[serde(untagged)]
pub enum RequestResponseBody<T> {
	Ok(T),
	Err(String),
}

impl<T> From<Result<T>> for RequestResponse<T> {
	fn from(res: Result<T>) -> Self {
		match res {
			Err(e) => {
				let e_ref: &(dyn std::error::Error) = e.as_ref();
				Self {
					status: e_ref.into(),
					body: RequestResponseBody::Err(e.to_string()),
				}
			}
			Ok(t) => Self {
				status: ResponseStatus::Success,
				body: RequestResponseBody::Ok(t),
			},
		}
	}
}
