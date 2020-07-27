use anyhow::Result;
use std::{
	convert::TryFrom,
	fmt::{self, Display, Formatter},
};
use uriparse::path::{Path, Segment};

#[derive(Debug)]
pub enum RouteError {
	RelativePath,
}

impl Display for RouteError {
	fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
		match self {
			Self::RelativePath => f.write_str("path is not absolute"),
		}
	}
}

impl std::error::Error for RouteError {}

pub fn make_route(path: &str) -> Result<String> {
	let mut path = Path::try_from(path)?;
	if !path.is_absolute() {
		return Err(RouteError::RelativePath.into());
	}

	let segments = path.segments_mut();
	match segments[0].as_str() {
		"guilds" | "channels" | "webhooks" if segments.len() > 1 => {
			segments[1] = Segment::try_from(":id").unwrap();
			Ok(path.into())
		}
		_ => Ok(path.into()),
	}
}

#[cfg(test)]
mod test {
	use super::make_route;

	#[test]
	fn makes_route() {
		assert_eq!(make_route("/foo/bar").unwrap(), "/foo/bar".to_string());
		assert_eq!(
			make_route("/guilds/1234/roles").unwrap(),
			"/guilds/:id/roles".to_string()
		);
	}
}
