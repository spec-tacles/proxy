#[cfg(feature = "redis-ratelimiter")]
#[macro_use]
extern crate log;
#[macro_use]
extern crate lazy_static;

use std::{future::Future, pin::Pin};

pub type DynFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;

pub mod cache;
pub mod models;
pub mod ratelimiter;
pub mod route;
