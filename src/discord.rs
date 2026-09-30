use crate::{
    i18n::tr,
    model::{Notification, Subscription, now},
    news::{excerpt, truncate},
    service::Service,
};
use anyhow::Result;
use poise::serenity_prelude as serenity;
use std::{fmt, sync::Arc, time::Duration};

pub type Data = Arc<Service>;
type Context<'a> = poise::Context<'a, Data, anyhow::Error>;

#[derive(Debug)]
struct UserError(String);
impl fmt::Display for UserError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for UserError {}
fn user_error(key: &str) -> anyhow::Error {
    UserError(tr(key, &[])).into()
}

#[derive(Clone, Copy, poise::ChoiceParameter)]
enum Mode {
    #[name = "builds"]
    Builds,
    #[name = "news"]
    News,
    #[name = "both"]
    Both,
}
impl Mode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Builds => "builds",
            Self::News => "news",
            Self::Both => "both",
        }
    }
}

pub fn commands() -> Vec<poise::Command<Data, anyhow::Error>> {
    fn configure(command: &mut poise::Command<Data, anyhow::Error>) {
        command.description = Some(tr(&format!("command.{}", command.name), &[]).into());
        command.default_member_permissions = serenity::Permissions::ADMINISTRATOR;
        command.required_permissions = serenity::Permissions::ADMINISTRATOR;
        command.guild_only = true;
        command.ephemeral = true;
        for parameter in &mut command.parameters {
            parameter.description = Some(tr(&format!("parameter.{}", parameter.name), &[]).into());
        }
        for child in &mut command.subcommands {
            configure(child);
        }
    }
    let mut root = steam();
    configure(&mut root);
    vec![root]
}

pub async fn run(service: Data) -> Result<()> {
    let token = service.config.token.clone();
    let guild = serenity::GuildId::new(service.config.guild_id);
    let framework = poise::Framework::builder()
        .options(poise::FrameworkOptions {
            commands: commands(),
            command_check: Some(|ctx| {
                Box::pin(async move {
                    if ctx.guild_id().map(|g| g.get()) != Some(ctx.data().config.guild_id) {
                        return Err(user_error("error.access"));
                    }
                    let admin = ctx
                        .author_member()
                        .await
                        .and_then(|m| m.permissions)
                        .is_some_and(|p| p.administrator());
                    if !admin {
                        return Err(user_error("error.access"));
                    }
                    Ok(true)
                })
            }),
            skip_checks_for_owners: false,
            on_error: |error| {
                Box::pin(async move {
                    if let Some(ctx) = error.ctx() {
                        let message = match &error {
                            poise::FrameworkError::Command { error, .. }
                            | poise::FrameworkError::CommandCheckFailed {
                                error: Some(error),
                                ..
                            } => error
                                .downcast_ref::<UserError>()
                                .map(|e| e.0.clone())
                                .unwrap_or_else(|| tr("error.generic", &[])),
                            poise::FrameworkError::MissingUserPermissions { .. } => {
                                tr("error.access", &[])
                            }
                            _ => tr("error.generic", &[]),
                        };
                        if let Err(send_error) = ctx
                            .send(
                                poise::CreateReply::default()
                                    .content(message)
                                    .ephemeral(true)
                                    .allowed_mentions(serenity::CreateAllowedMentions::new()),
                            )
                            .await
                        {
                            tracing::warn!(%send_error, "Failed to send command error");
                        }
                    }
                    tracing::warn!(%error, "Discord command failed");
                })
            },
            ..Default::default()
        })
        .setup(move |ctx, _ready, framework| {
            Box::pin(async move {
                poise::builtins::register_in_guild(ctx, &framework.options().commands, guild)
                    .await?;
                tracing::info!(guild_id = guild.get(), "Discord commands registered");
                Ok(service)
            })
        })
        .build();
    let mut client = serenity::ClientBuilder::new(token, serenity::GatewayIntents::GUILDS)
        .framework(framework)
        .await?;
    let manager = client.shard_manager.clone();
    tokio::select! {
        result = client.start() => result?,
        _ = shutdown_signal() => { manager.shutdown_all().await; }
    }
    Ok(())
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .expect("install SIGTERM handler");
        tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = terminate.recv() => {} }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

async fn reply(ctx: Context<'_>, text: String) -> Result<()> {
    ctx.send(
        poise::CreateReply::default()
            .content(text)
            .ephemeral(true)
            .allowed_mentions(serenity::CreateAllowedMentions::new()),
    )
    .await?;
    Ok(())
}

async fn reply_lines(ctx: Context<'_>, lines: Vec<String>) -> Result<()> {
    let mut page = String::new();
    for line in lines {
        let line = truncate(&line, 1700);
        if page.encode_utf16().count() + line.encode_utf16().count() + 1 > 1800 {
            reply(ctx, std::mem::take(&mut page)).await?;
        }
        page.push_str(&line);
        page.push('\n');
    }
    if !page.is_empty() {
        reply(ctx, page).await?;
    }
    Ok(())
}

async fn client(ctx: Context<'_>) -> Result<crate::steam::Steam> {
    ctx.data()
        .steam
        .read()
        .await
        .clone()
        .ok_or_else(|| user_error("error.steam"))
}

fn app_id(value: i64) -> Result<u32> {
    u32::try_from(value)
        .ok()
        .filter(|n| *n > 0)
        .ok_or_else(|| user_error("error.id"))
}

async fn validate_destination(
    ctx: Context<'_>,
    channel: &serenity::GuildChannel,
    role: Option<serenity::RoleId>,
) -> Result<()> {
    if channel.guild_id.get() != ctx.data().config.guild_id
        || !matches!(
            channel.kind,
            serenity::ChannelType::Text | serenity::ChannelType::News
        )
    {
        return Err(user_error("error.channel"));
    }
    let guild = channel.guild_id.to_partial_guild(ctx.http()).await?;
    let bot_id = ctx.serenity_context().cache.current_user().id;
    let member = channel.guild_id.member(ctx.http(), bot_id).await?;
    let permissions = guild.user_permissions_in(channel, &member);
    if !permissions.contains(
        serenity::Permissions::VIEW_CHANNEL
            | serenity::Permissions::SEND_MESSAGES
            | serenity::Permissions::EMBED_LINKS
            | serenity::Permissions::ATTACH_FILES,
    ) {
        return Err(user_error("error.permissions"));
    }
    if let Some(role_id) = role {
        let valid = role_id.get() != guild.id.get()
            && guild
                .roles
                .get(&role_id)
                .is_some_and(|r| r.mentionable || permissions.mention_everyone());
        if !valid {
            return Err(user_error("error.role"));
        }
    }
    Ok(())
}

async fn prepare(ctx: Context<'_>, sub: &mut Subscription) -> Result<Vec<crate::news::Article>> {
    app_id(sub.news_app_id)?;
    let steam = client(ctx).await?;
    let app = steam
        .app(app_id(sub.app_id)?)
        .await
        .map_err(|_| user_error("error.app"))?;
    sub.name = app.name;
    sub.icon_url = app.icon_url;
    if sub.builds() {
        sub.build_id = Some(
            app.branches
                .get(&sub.branch)
                .filter(|b| !b.password_required)
                .ok_or_else(|| user_error("error.branch"))?
                .build_id
                .clone(),
        );
    } else {
        sub.build_id = None;
    }
    if sub.news() {
        let source = app_id(sub.news_app_id)?;
        if source != app.id {
            steam
                .app(source)
                .await
                .map_err(|_| user_error("error.app"))?;
        }
        ctx.data().news.articles(source, now() - 86400).await
    } else {
        Ok(Vec::new())
    }
}

async fn save(
    ctx: Context<'_>,
    sub: &Subscription,
    articles: &[crate::news::Article],
) -> Result<i64> {
    let _lock = ctx.data().mutations.lock().await;
    if ctx.data().db.list().await?.iter().any(|s| {
        s.id != sub.id
            && s.app_id == sub.app_id
            && s.branch == sub.branch
            && s.channel_id == sub.channel_id
    }) {
        return Err(user_error("error.duplicate"));
    }
    let id = ctx.data().db.save(sub, Some(articles)).await?;
    ctx.data().clear(&format!("build:{id}")).await;
    ctx.data().clear(&format!("news:{id}")).await;
    Ok(id)
}

#[poise::command(
    slash_command,
    subcommands(
        "follow", "edit", "remove", "list", "branches", "dayz", "status", "test"
    )
)]
async fn steam(_ctx: Context<'_>) -> Result<()> {
    Ok(())
}

#[poise::command(slash_command)]
async fn follow(
    ctx: Context<'_>,
    app_id: i64,
    channel: serenity::GuildChannel,
    mode: Option<Mode>,
    branch: Option<String>,
    role: Option<serenity::Role>,
    news_app_id: Option<i64>,
) -> Result<()> {
    ctx.defer_ephemeral().await?;
    validate_destination(ctx, &channel, role.as_ref().map(|r| r.id)).await?;
    let mut sub = Subscription {
        id: 0,
        app_id,
        name: String::new(),
        icon_url: None,
        branch: branch.unwrap_or_else(|| "public".into()),
        mode: mode.unwrap_or(Mode::Both).as_str().into(),
        news_app_id: news_app_id.unwrap_or(app_id),
        channel_id: channel.id.to_string(),
        role_id: role.map(|r| r.id.to_string()),
        build_id: None,
        news_initialized: false,
        revision: 0,
    };
    let articles = prepare(ctx, &mut sub).await?;
    let id = save(ctx, &sub, &articles).await?;
    reply(
        ctx,
        tr(
            "reply.saved",
            &[("id", &id.to_string()), ("name", &excerpt(&sub.name))],
        ),
    )
    .await
}

#[poise::command(slash_command)]
#[allow(clippy::too_many_arguments)]
async fn edit(
    ctx: Context<'_>,
    id: i64,
    channel: Option<serenity::GuildChannel>,
    mode: Option<Mode>,
    branch: Option<String>,
    role: Option<serenity::Role>,
    clear_role: Option<bool>,
    news_app_id: Option<i64>,
) -> Result<()> {
    ctx.defer_ephemeral().await?;
    let mut sub = ctx
        .data()
        .db
        .get(id)
        .await?
        .ok_or_else(|| user_error("error.missing"))?;
    if clear_role == Some(true) && role.is_some() {
        return Err(user_error("error.role_conflict"));
    }
    if let Some(mode) = mode {
        sub.mode = mode.as_str().into();
    }
    if let Some(branch) = branch {
        sub.branch = branch;
    }
    if let Some(source) = news_app_id {
        sub.news_app_id = source;
    }
    if clear_role == Some(true) {
        sub.role_id = None;
    }
    if let Some(role) = role {
        sub.role_id = Some(role.id.to_string());
    }
    let channel = match channel {
        Some(channel) => channel,
        None => serenity::ChannelId::new(sub.channel_id.parse()?)
            .to_channel(ctx.http())
            .await?
            .guild()
            .ok_or_else(|| user_error("error.channel"))?,
    };
    validate_destination(
        ctx,
        &channel,
        sub.role_id
            .as_ref()
            .map(|r| r.parse().map(serenity::RoleId::new))
            .transpose()?,
    )
    .await?;
    sub.channel_id = channel.id.to_string();
    let articles = prepare(ctx, &mut sub).await?;
    save(ctx, &sub, &articles).await?;
    reply(
        ctx,
        tr(
            "reply.saved",
            &[("id", &id.to_string()), ("name", &excerpt(&sub.name))],
        ),
    )
    .await
}

#[poise::command(slash_command)]
async fn remove(ctx: Context<'_>, id: i64) -> Result<()> {
    ctx.defer_ephemeral().await?;
    let _lock = ctx.data().mutations.lock().await;
    if !ctx.data().db.remove(id).await? {
        return Err(user_error("error.missing"));
    }
    ctx.data().clear(&format!("build:{id}")).await;
    ctx.data().clear(&format!("news:{id}")).await;
    reply(ctx, tr("reply.removed", &[("id", &id.to_string())])).await
}

#[poise::command(slash_command)]
async fn list(ctx: Context<'_>) -> Result<()> {
    ctx.defer_ephemeral().await?;
    let subs = ctx.data().db.list().await?;
    if subs.is_empty() {
        return reply(ctx, tr("reply.empty", &[])).await;
    }
    let lines = subs
        .iter()
        .map(|s| {
            tr(
                "reply.row",
                &[
                    ("id", &s.id.to_string()),
                    ("name", &excerpt(&s.name)),
                    ("app_id", &s.app_id.to_string()),
                    ("branch", &excerpt(&s.branch)),
                    ("mode", &s.mode),
                    ("channel", &s.channel_id),
                    ("news_app_id", &s.news_app_id.to_string()),
                    (
                        "role",
                        &s.role_id.clone().unwrap_or_else(|| tr("status.none", &[])),
                    ),
                ],
            )
        })
        .collect();
    reply_lines(ctx, lines).await
}

#[poise::command(slash_command)]
async fn branches(ctx: Context<'_>, app_id: i64) -> Result<()> {
    ctx.defer_ephemeral().await?;
    let app = client(ctx)
        .await?
        .app(crate::discord::app_id(app_id)?)
        .await
        .map_err(|_| user_error("error.app"))?;
    let lines: Vec<_> = app
        .branches
        .iter()
        .filter(|(_, b)| !b.password_required)
        .map(|(name, b)| {
            tr(
                "reply.branch",
                &[("branch", &excerpt(name)), ("build", &b.build_id)],
            )
        })
        .collect();
    if lines.is_empty() {
        reply(ctx, tr("reply.no_branches", &[])).await
    } else {
        reply_lines(ctx, lines).await
    }
}

#[poise::command(slash_command)]
async fn dayz(
    ctx: Context<'_>,
    channel: serenity::GuildChannel,
    role: Option<serenity::Role>,
) -> Result<()> {
    ctx.defer_ephemeral().await?;
    validate_destination(ctx, &channel, role.as_ref().map(|r| r.id)).await?;
    let steam = client(ctx).await?;
    let mut added = 0;
    let mut skipped = 0;
    for id in [221100, 1024020, 223350, 1042420] {
        if ctx.data().db.list().await?.iter().any(|s| {
            s.app_id == id && s.branch == "public" && s.channel_id == channel.id.to_string()
        }) {
            skipped += 1;
            continue;
        }
        let app = steam
            .app(id as u32)
            .await
            .map_err(|_| user_error("error.app"))?;
        let client_app = matches!(id, 221100 | 1024020);
        let mut sub = Subscription {
            id: 0,
            app_id: id,
            name: app.name,
            icon_url: app.icon_url,
            branch: "public".into(),
            mode: if client_app { "both" } else { "builds" }.into(),
            news_app_id: if client_app { 221100 } else { id },
            channel_id: channel.id.to_string(),
            role_id: role.as_ref().map(|r| r.id.to_string()),
            build_id: app
                .branches
                .get("public")
                .filter(|b| !b.password_required)
                .map(|b| b.build_id.clone()),
            news_initialized: false,
            revision: 0,
        };
        let articles = if client_app {
            ctx.data()
                .news
                .articles(sub.news_app_id as u32, now() - 86400)
                .await?
        } else {
            Vec::new()
        };
        sub.id = save(ctx, &sub, &articles).await?;
        if sub.build_id.is_none() {
            ctx.data()
                .error(
                    &format!("build:{}", sub.id),
                    tr("health.preset_branch", &[("app_id", &id.to_string())]),
                )
                .await;
        }
        added += 1;
    }
    reply(
        ctx,
        tr(
            "reply.dayz",
            &[
                ("added", &added.to_string()),
                ("skipped", &skipped.to_string()),
            ],
        ),
    )
    .await
}

#[poise::command(slash_command)]
async fn status(ctx: Context<'_>) -> Result<()> {
    ctx.defer_ephemeral().await?;
    let pending = ctx.data().db.pending_count().await?;
    let failed_channels = ctx.data().db.failed_channels().await?;
    let health = ctx.data().health.read().await;
    let time = |value: Option<i64>| {
        value
            .map(|t| format!("<t:{t}:R>"))
            .unwrap_or_else(|| tr("status.never", &[]))
    };
    let mut error_lines: Vec<_> = health
        .errors
        .iter()
        .map(|(k, v)| format!("{k}: {v}"))
        .collect();
    error_lines.extend(
        failed_channels
            .iter()
            .map(|channel| tr("health.delivery_channel", &[("channel", channel)])),
    );
    let errors = if error_lines.is_empty() {
        tr("status.none", &[])
    } else {
        error_lines.join("\n")
    };
    reply(
        ctx,
        tr(
            "status.body",
            &[
                (
                    "steam",
                    &tr(
                        if health.steam_connected {
                            "status.connected"
                        } else {
                            "status.disconnected"
                        },
                        &[],
                    ),
                ),
                ("build", &time(health.last_build)),
                ("news", &time(health.last_news)),
                ("pending", &pending.to_string()),
                ("errors", &truncate(&errors, 1200)),
            ],
        ),
    )
    .await
}

#[poise::command(slash_command)]
async fn test(
    ctx: Context<'_>,
    channel: serenity::GuildChannel,
    mode: Option<Mode>,
    app_id: Option<i64>,
) -> Result<()> {
    ctx.defer_ephemeral().await?;
    validate_destination(ctx, &channel, None).await?;
    let mut events = Vec::new();
    if let (Some(m), Some(id)) = (mode, app_id) {
        let steam = client(ctx).await?;
        let app = steam
            .app(crate::discord::app_id(id)?)
            .await
            .map_err(|_| user_error("error.app"))?;
        if matches!(m, Mode::Builds | Mode::Both) {
            events.push(Notification::Build {
                name: app.name.clone(),
                app_id: id as u32,
                branch: "public".into(),
                old: "1234567".into(),
                new: "1234568".into(),
                detected_at: crate::model::now(),
                icon_url: app.icon_url.clone(),
            });
        }
        if matches!(m, Mode::News | Mode::Both) {
            if let Some(article) = ctx.data().news.latest_article(id as u32).await? {
                let url = article.safe_url();
                let mut images = crate::presentation::article(&article.contents).images;
                if let Some(cover) = article.cover_image
                    && !images.contains(&cover)
                {
                    images.insert(0, cover);
                }
                events.push(Notification::News {
                    title: article.title,
                    excerpt: crate::news::excerpt(&article.contents),
                    url,
                    published_at: article.date,
                    images,
                });
            }
        }
    }
    if events.is_empty() {
        events.push(Notification::Test);
    }
    for event in events {
        for (mut outgoing, files) in messages(&event, None) {
            for (index, url) in files.iter().enumerate() {
                match ctx.data().media.download(url, index).await {
                    Ok(attachment) => outgoing = outgoing.add_file(attachment),
                    Err(error) => {
                        tracing::warn!(%error, "Skipping unavailable announcement image in test")
                    }
                }
            }
            channel.id.send_message(ctx.http(), outgoing).await?;
        }
    }
    reply(ctx, tr("reply.test", &[])).await
}

pub fn messages(
    event: &Notification,
    role: Option<u64>,
) -> Vec<(serenity::CreateMessage, Vec<String>)> {
    let role = if matches!(event, Notification::Test) {
        None
    } else {
        role
    };
    let mention = role.map(|r| format!("<@&{r}>")).unwrap_or_default();
    let allowed = serenity::CreateAllowedMentions::new()
        .everyone(false)
        .all_users(false)
        .all_roles(false)
        .roles(role.map(serenity::RoleId::new))
        .replied_user(false);
    let base = serenity::CreateMessage::new()
        .content(mention.clone())
        .allowed_mentions(allowed.clone());
    match event {
        Notification::News {
            title,
            excerpt,
            url,
            published_at,
            images,
        } => {
            let title = truncate(
                &crate::presentation::escape(&title.replace(['\n', '\r'], " ")),
                150,
            );
            let source = if url.len() <= 400 {
                url.as_str()
            } else {
                "https://steamcommunity.com/"
            };
            let heading = tr(
                "news.heading",
                &[("title", &title), ("time", &published_at.to_string())],
            );
            let footer = tr("news.footer", &[("url", source)]);
            let prefix = if mention.is_empty() {
                heading
            } else {
                format!("{mention}\n{heading}")
            };
            let budget = 2000_usize.saturating_sub(footer.encode_utf16().count());
            let body = crate::presentation::preview(excerpt, budget);
            
            let mut msgs = Vec::new();
            
            // Message 1: Heading + URL + Cover Image
            let msg1 = serenity::CreateMessage::new()
                .content(format!("{prefix}\n<{source}>"))
                .allowed_mentions(allowed.clone());
            let mut msg1_images = Vec::new();
            if let Some(cover) = images.first() {
                msg1_images.push(cover.clone());
            }
            msgs.push((msg1, msg1_images));

            // Message 2: Summary + Inline Images
            if !body.is_empty() {
                let msg2 = serenity::CreateMessage::new()
                    .content(format!("{body}{footer}"))
                    .flags(serenity::MessageFlags::SUPPRESS_EMBEDS)
                    .allowed_mentions(allowed);
                let msg2_images = images.iter().skip(1).take(3).cloned().collect();
                msgs.push((msg2, msg2_images));
            }

            msgs
        }
        Notification::Build {
            name,
            app_id,
            branch,
            old,
            new,
            detected_at,
            icon_url,
        } => {
            let mut embed = serenity::CreateEmbed::new()
                .author(serenity::CreateEmbedAuthor::new(tr("build.label", &[])))
                .title(truncate(&crate::presentation::escape(name), 200))
                .url(format!("https://steamcommunity.com/app/{app_id}"))
                .description(tr(
                    "build.description",
                    &[
                        ("branch", &truncate(&excerpt(branch), 100)),
                        ("time", &detected_at.to_string()),
                    ],
                ))
                .field(
                    tr("build.previous", &[]),
                    format!("`{}`", truncate(old, 50)),
                    true,
                )
                .field(
                    tr("build.current", &[]),
                    format!("`{}`", truncate(new, 50)),
                    true,
                )
                .footer(serenity::CreateEmbedFooter::new(tr(
                    "build.footer",
                    &[("app_id", &app_id.to_string())],
                )))
                .color(0x57f287);
            if let Some(url) = icon_url.as_deref().and_then(crate::presentation::image_url) {
                embed = embed.thumbnail(url);
            }
            vec![(base.content(mention).embed(embed), Vec::new())]
        }
        Notification::Test => vec![(base.embed(
            serenity::CreateEmbed::new()
                .title(tr("test.title", &[]))
                .description(tr("test.body", &[]))
                .color(0xfee75c),
        ), Vec::new())],
    }
}

pub async fn delivery_worker(service: Data) {
    let http = serenity::Http::new(&service.config.token);
    loop {
        if let Err(error) = deliver_pending(&service, &http).await {
            tracing::warn!(%error, "Delivery queue failed");
            service.error("delivery", tr("health.delivery", &[])).await;
        }
        tokio::time::sleep(Duration::from_secs(3)).await;
    }
}

async fn deliver_pending(service: &Service, http: &serenity::Http) -> Result<()> {
    for candidate in service.db.pending().await? {
        let _lock = service.mutations.lock().await;
        let Some(delivery) = service.db.pending_delivery(candidate.id).await? else {
            continue;
        };
        let event: Notification = serde_json::from_str(&delivery.payload)?;
        let channel = serenity::ChannelId::new(delivery.channel_id.parse()?);
        let role = delivery.role_id.as_ref().map(|r| r.parse()).transpose()?;
        let msgs = messages(&event, role);
        let mut all_sent = true;
        for (mut outgoing, files) in msgs {
            for (index, url) in files.iter().enumerate() {
                match service.media.download(url, index).await {
                    Ok(attachment) => outgoing = outgoing.add_file(attachment),
                    Err(error) => tracing::warn!(%error, "Skipping unavailable announcement image"),
                }
            }
            match tokio::time::timeout(
                Duration::from_secs(45),
                channel.send_message(http, outgoing),
            )
            .await
            {
                Ok(Ok(_)) => {}
                result => {
                    let error = match result {
                        Ok(Err(error)) => error.to_string(),
                        Err(_) => "Discord delivery timed out".into(),
                        _ => unreachable!(),
                    };
                    tracing::warn!(delivery_id=delivery.id, %error, "Discord notification failed");
                    service.db.failed(&delivery, &error).await?;
                    all_sent = false;
                    break;
                }
            }
        }
        if all_sent {
            service.db.delivered(delivery.id).await?;
        }
    }
    service.clear("delivery").await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn command_catalog_and_english_identifiers() {
        let commands = commands();
        let names: Vec<_> = commands[0]
            .subcommands
            .iter()
            .map(|c| c.name.as_ref())
            .collect();
        assert_eq!(
            names,
            [
                "follow", "edit", "remove", "list", "branches", "dayz", "status", "test"
            ]
        );
        assert!(commands[0].create_as_slash_command().is_some());
        for command in &commands[0].subcommands {
            assert!(command.required_permissions.administrator());
            assert!(command.description.as_ref().unwrap().len() <= 100);
            for p in &command.parameters {
                assert!(p.description.is_some());
            }
        }
    }
    #[test]
    fn test_never_pings_and_timestamps_are_native() {
        let json = serde_json::to_value(messages(&Notification::Test, Some(123))[0].0.clone()).unwrap();
        assert_eq!(json["content"], "");
        assert_eq!(json["allowed_mentions"]["parse"], serde_json::json!([]));
        
        let event = Notification::Build {
            name: "Café".into(),
            app_id: 42,
            branch: "public".into(),
            old: "1".into(),
            new: "2".into(),
            detected_at: 123,
            icon_url: Some(
                "https://cdn.akamai.steamstatic.com/steamcommunity/public/images/apps/42/icon.jpg"
                    .into(),
            ),
        };
        let json = serde_json::to_value(messages(&event, Some(456))[0].0.clone()).unwrap();
        assert_eq!(json["content"], "<@&456>");
        assert_eq!(
            json["embeds"][0]["thumbnail"]["url"],
            "https://cdn.akamai.steamstatic.com/steamcommunity/public/images/apps/42/icon.jpg"
        );
        assert_eq!(json["embeds"][0]["fields"][1]["name"], "New build");
        assert!(
            json["embeds"][0]["description"]
                .as_str()
                .unwrap()
                .contains("<t:123:F>")
        );
    }

    #[test]
    fn announcements_are_plain_messages_with_bounded_unicode_content() {
        let event = Notification::News {
            title: "Update @everyone".into(),
            excerpt: "### Fixes\n- Fixed collision\n\n".repeat(200),
            url: "https://steamcommunity.com/games/42/announcements/detail/123".into(),
            published_at: 123,
            images: vec![],
        };
        let msgs = messages(&event, Some(u64::MAX));
        let json_msg1 = serde_json::to_value(msgs[0].0.clone()).unwrap();
        let content1 = json_msg1["content"].as_str().unwrap();
        assert!(content1.starts_with("<@&18446744073709551615>\n## Update @\u{200b}everyone"));
        
        let json_msg2 = serde_json::to_value(msgs[1].0.clone()).unwrap();
        let content2 = json_msg2["content"].as_str().unwrap();
        assert!(content2.contains("- Fixed collision"));
        assert!(content2.contains("Read full announcement"));
        assert!(content2.encode_utf16().count() <= 2000);
        assert!(json_msg2.get("embeds").is_none() || json_msg2["embeds"].as_array().unwrap().is_empty());
        assert_eq!(json_msg2["flags"], 4);
    }

    #[test]
    fn existing_queued_events_remain_readable() {
        let old_news = r#"{"kind":"News","title":"Update","excerpt":"Details","url":"https://steamcommunity.com/","published_at":123}"#;
        let old_build = r#"{"kind":"Build","name":"DayZ","app_id":221100,"branch":"public","old":"1","new":"2","detected_at":123}"#;
        for value in [old_news, old_build] {
            let event: Notification = serde_json::from_str(value).unwrap();
            let _ = messages(&event, None);
        }
    }
}
