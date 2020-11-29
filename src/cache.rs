use crate::DynFuture;
use anyhow::Result;
use rustacles_model::{
	channel::Channel, guild::Guild, message::Message, presence::Presence, voice::VoiceState,
	Snowflake,
};

pub mod redis;

pub type FutureResultOption<T> = DynFuture<Result<Option<T>>>;
pub type FutureEmptyResult = DynFuture<Result<()>>;

pub trait Cache<T> {
	fn get(&self, id: Snowflake) -> FutureResultOption<T>;
	fn save(&self, item: T) -> FutureEmptyResult;
	fn delete(&self, id: Snowflake) -> FutureEmptyResult;
}

pub trait DiscordCache:
	Cache<Channel> + Cache<Guild> + Cache<Message> + Cache<Presence> + Cache<VoiceState>
{
}
