use anyhow::{anyhow, Result};
use std::convert::TryFrom;
use uriparse::path::{Path, Segment};

pub fn make_route(path: &str) -> Result<String> {
	let mut path = Path::try_from(path)?;
	if !path.is_absolute() {
		return Err(anyhow!("path is not absolute"));
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
