
use crate::database;
use serenity::{
    model::{id::{ChannelId, GuildId, UserId},
            event::PresenceUpdateEvent,
            gateway::{ActivityType, Activity},
    },
    prelude::*,
    utils::MessageBuilder,
};
use std::env;
use time;

const DEFAULT_STREAM_NOTIFY_COOLDOWN: i64 = 21600; // 6 hours

/// Handler that decides whether the updating of the presence of a guild member
/// should result in the sending of a "shout-out" message in that guild,
/// and sends that message if required.
pub fn handler(
    context: Context,
    presence_update_event: PresenceUpdateEvent
) {
    // Stream start detection
    debug!("In stream_notify::handler: presence_update_event.presence.activity = {:?}",
           presence_update_event.presence.activity);
    let streaming_activity = match presence_update_event.presence.activity {
        None => {
            debug!("No activity in presence update, ignoring");
            return
        }
        Some(activity) => match activity.kind {
            ActivityType::Streaming => activity,
            _ => {
                debug!("Activity in presence update is not a stream, ignoring");
                return
            }
        },
    };

    debug!("User Discord data retrieval...");
    let user_id = presence_update_event.presence.user_id;

    match context.cache.read().user(user_id) {
        None => {
            error!("Failed to get Discord user object from Serenity cache");
            return
        },
        Some(user) => {
            let user = user.read();
            debug!("Member {} ({}#{}) presence update with activity name '{:?}'",
                   user.id,
                   user.name,
                   user.discriminator,
                   streaming_activity.name,
            );
        },
    };
    let database = database::Handle::new();

    debug!("Guild ID retrieval...");
    let guild_id = match presence_update_event.guild_id {
        None => {
            debug!("Got presence update with no discord guild ID");
            return
        },
        Some(guild_id) => guild_id,
    };

    debug!("Member DB data retrieval...");
    let member: database::Member = match database.member(
        guild_id.0, user_id.0)
    {
        Err(_) => {
            error!("Could not retrieve member data from database");
            return
        },
        Ok(data) => data,
    };

    // Set STREAM_NOTIFY_COOLDOWN in the mount/env file to override the default duration.
    let stream_notify_cooldown: i64 = match env::var("STREAM_NOTIFY_COOLDOWN") {
        Ok(val_s) => match val_s.parse() {
            Ok(val) => val,
            Err(_) => DEFAULT_STREAM_NOTIFY_COOLDOWN,
        },
        Err(_) => DEFAULT_STREAM_NOTIFY_COOLDOWN
    };
    debug!("Using stream advertise cooldown = {} seconds",
           stream_notify_cooldown);

    debug!("Member data: {:?}", member);
    if time::get_time().sec - member.last_stream_notify_timestamp > stream_notify_cooldown {
        // We will shout out the stream
        stream_notify(context, member, guild_id, user_id, streaming_activity);
    } else {
        debug!("Last stream too recent; would not shout out stream");
    }
}

fn stream_notify(
    context: Context,
    member: database::Member,
    guild_id: GuildId,
    user_id: UserId,
    streaming_activity: Activity,
) {
    debug!("User DB data retrieval...");
    let database = database::Handle::new();
    let user: database::User = match database.user(user_id.0)
    {
        Err(_) => {
            error!("Could not retrieve user data from database");
            return
        },
        Ok(data) => data,
    };

    let user_title = match user.title {
        None => String::new(),
        Some(prefix) => {
            let mut prefix = prefix.clone();
            prefix.push(' ');
            prefix
        },
    };

    // Update the timestamp of the last shout-out in the database
    let mut member = member.clone();
    member.last_stream_notify_timestamp = time::get_time().sec;
    if let Err(_) = database.member_update(
        guild_id.0,
        user_id.0,
        &member
    ) {
        error!("Could update member data in database");
        return
    }
    debug!("Updated member timestamp to {}", member.last_stream_notify_timestamp);

    // Set STREAM_NOTIFY_CHANNEL_ID in the mount/env file to the name of a channel in the guild.
    // By using the ID, the channel can be renamed without breaking the integration.
    // TODO: Make this a DB setting in the Guilds table instead, since different guilds will
    // want to use different channel names for this.
    let discord_channel_id = match env::var("STREAM_NOTIFY_CHANNEL_ID")
    {
        Ok(channel_id) => match channel_id.parse::<u64>() {
            Ok(val) => ChannelId(val),
            Err(_) => {
                error!("STREAM_NOTIFY_CHANNEL_ID is invalid, can't send stream notification");
                return
            }
        },
        Err(_) => {
            error!("STREAM_NOTIFY_CHANNEL_ID is not set, can't send stream notification");
            return
        }
    };

    let discord_guild = match context.cache.read().guild(guild_id) {
        Some(guild) => guild,
        None => {
            error!("Could not retrieve guild from Serenity cache");
            return
        }
    };
    let discord_guild = discord_guild.read();

    let discord_channel = match discord_guild
        .channels
        .get(&discord_channel_id)
    {
        Some(channel) => channel.read(),
        None => {
            error!("Could not retrieve guild channel from Serenity cache");
            return
        }
    };

    // Get the member name in the context of the guild (there could be a nickname)
    let member_name = match discord_guild
        .members
        .get(&user_id)
    {
        Some(member) => member.display_name(),
        None => {
            error!("Could not retrieve guild member from Serenity cache");
            return
        }
    };

    let stream_url = match streaming_activity.url {
        Some(url) => url,
        None => {
            error!("No stream URL found in presence update");
            return
        }
    };

    let response = MessageBuilder::new()
        .push(user_title)
        .push_bold_safe(member_name)
        .push(" is streaming ")
        .push_bold_safe(streaming_activity.name)
        .push(":\n")
        .push(stream_url)
        .build();

    if let Err(why) = discord_channel.say(&context.http, &response) {
        error!("Error sending message: {:?}", why);
    }
}
