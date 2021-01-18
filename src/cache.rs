use anyhow::Result;
use async_trait::async_trait;
use rustacles_model::{
	channel::Channel, guild::Guild, message::Message, presence::Presence, voice::VoiceState,
	Snowflake,
};

pub mod redis;

#[async_trait]
pub trait Cache<T> {
	async fn get(&self, id: Snowflake) -> Result<Option<T>>;
	async fn save(&self, item: T) -> Result<()>;
	async fn delete(&self, id: Snowflake) -> Result<()>;
}

pub trait DiscordCache:
	Cache<Channel> + Cache<Guild> + Cache<Message> + Cache<Presence> + Cache<VoiceState>
{
}
