use serenity::{
    client::Context,
    model::{
        guild::{Guild, PartialMember},
        permissions::Permissions,
    },
    utils::Colour,
    Result as SerenityResult,
};
use tracing::{warn, info};

pub fn handle_message<T>(res: SerenityResult<T>) {
    match res {
        Ok(_) => (),
        Err(e) => warn!("Could not send/delete message: {}", e),
    }
}

#[cfg(feature = "cache")]
pub fn handle_io<T>(res: std::io::Result<T>) -> T {
    match res {
        Ok(t) => t,
        Err(e) => { tracing::error!("I/O error: {}", e); panic!() },
    }
}

pub async fn permission_check(ctx: &Context, mem: &PartialMember) -> bool {
    for role in &mem.roles {
        if role.to_role_cached(&ctx.cache).await.map_or(false, |r| {
            r.has_permission(Permissions::MANAGE_CHANNELS) || r.name.to_lowercase() == "dj"
        }) {
            return true;
        }
    }
    info!(
        "Permission denied for user {}",
        mem.nick.clone().unwrap_or("?".to_owned())
    );
    return false;
}

pub async fn cached_colour(ctx: &Context, guild: Option<Guild>) -> Colour {
    if let Some(g) = guild {
        if let Ok(me) = g.member(&ctx, ctx.cache.current_user_id().await).await {
            return me.colour(&ctx.cache).await.unwrap_or(Colour(0xffffff));
        }
    };
    Colour(0xffffff)
}
