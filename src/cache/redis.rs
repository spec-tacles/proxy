use super::{Cache, DiscordCache};
use anyhow::Result;
use async_trait::async_trait;
use lazy_static::lazy_static;
use redis::{Client, Script};
use rustacles_model::{channel::Channel, guild::Guild, Snowflake};
use serde_json::{from_str, to_vec};

lazy_static! {
	static ref SAVE_GUILD: Script = Script::new(include_str!("scripts/save_guild.lua"));
	static ref DELETE_GUILD: Script = Script::new(include_str!("scripts/delete_guild.lua"));
}

#[async_trait]
impl Cache<Guild> for Client {
	async fn get(&self, id: Snowflake) -> Result<Option<Guild>> {
		let redis = self.clone();
		let guild_str: Option<String> = redis::cmd("JSON.GET")
			.arg(format!("guilds.{}", id))
			.arg(".")
			.query_async(&mut redis.get_async_connection().await?)
			.await?;

		Ok(guild_str.map(|s| from_str(&s)).transpose()?)
	}

	async fn save(&self, item: Guild) -> Result<()> {
		let redis = self.clone();
		let guild_vec = to_vec(&item)?;
		let mut cmd = SAVE_GUILD.key(format!("guilds.{}", item.id));
		cmd.arg(guild_vec);

		for channel in item.channels {
			let channel_vec = to_vec(&channel)?;
			cmd.key(format!("channels.{}", channel.id)).arg(channel_vec);
		}

		cmd.invoke_async::<_, redis::Value>(&mut redis.get_async_connection().await?)
			.await?;
		Ok(())
	}

	async fn delete(&self, id: Snowflake) -> Result<()> {
		let redis = self.clone();

		let maybe_guild: Option<Guild> = Cache::<Guild>::get(&redis, id).await?;

		if let Some(guild) = maybe_guild {
			let mut cmd = DELETE_GUILD.key(format!("guilds.{}", guild.id));
			for channel in guild.channels {
				cmd.key(format!("channels.{}", channel.id));
			}

			cmd.invoke_async::<_, redis::Value>(&mut redis.get_async_connection().await?)
				.await?;
		}

		Ok(())
	}
}

#[async_trait]
impl Cache<Channel> for Client {
	async fn get(&self, id: Snowflake) -> Result<Option<Channel>> {
		todo!()
	}

	async fn save(&self, item: Channel) -> Result<()> {
		todo!()
	}

	async fn delete(&self, id: Snowflake) -> Result<()> {
		todo!()
	}
}

// impl DiscordCache for RedisCache {}
