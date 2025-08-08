use std::{sync::Arc, time::Duration};

use anyhow::Result;
use log::{debug, error};
use serenity::all::{
    CacheHttp, Command, CommandDataOption, CommandInteraction, CommandOptionType, Context,
    CreateCommand, CreateCommandOption, EditInteractionResponse, EventHandler, GatewayIntents,
    Interaction, Ready, ShardManager,
};
use serenity::{Client, async_trait};
use strum::{Display, EnumIter, EnumMessage, EnumString, IntoEnumIterator};
use tokio::{
    runtime::Handle,
    spawn,
    sync::{
        Mutex,
        mpsc::{Receiver, Sender, channel},
        oneshot,
    },
    task::{JoinHandle, block_in_place},
    time::{Instant, sleep, timeout},
};

#[derive(Debug, Clone, Copy)]
pub enum BotCommandKind {
    Start,
    Stop,
    Status,
    Chat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, EnumIter, EnumString, EnumMessage, Display)]
enum BotCommandKindInner {
    #[strum(to_string = "start", message = "Start the bot actions")]
    Start,
    #[strum(to_string = "stop", message = "Stop the bot actions")]
    Stop,
    #[strum(to_string = "status", message = "See bot current status")]
    Status,
    #[strum(
        to_string = "chat",
        message = "Send a message inside the game (256 characters max)"
    )]
    Chat,
    #[strum(
        to_string = "start-stream",
        message = "Start streaming game images over time (15 minutes max)"
    )]
    StartStream,
    #[strum(to_string = "stop-stream", message = "Stop streaming game images")]
    StopStream,
}

#[derive(Debug)]
pub struct BotCommand {
    pub kind: BotCommandKind,
    pub options: Vec<CommandDataOption>,
    pub sender: oneshot::Sender<EditInteractionResponse>,
}

#[derive(Debug)]
pub struct DiscordBot {
    command_sender: Sender<BotCommand>,
    shard_manager: Option<Arc<ShardManager>>,
}

impl DiscordBot {
    pub fn new() -> (Self, Receiver<BotCommand>) {
        let (tx, rx) = channel(3);
        let bot = Self {
            command_sender: tx,
            shard_manager: None,
        };

        (bot, rx)
    }

    pub fn start(&mut self, token: String) -> Result<()> {
        self.shutdown();

        let sender = self.command_sender.clone();
        let handler = DefaultEventHandler {
            command_sender: sender,
            stream_handle: Arc::new(Mutex::new(None)),
        };

        let builder = Client::builder(token, GatewayIntents::empty()).event_handler(handler);
        let mut client =
            block_in_place(move || Handle::current().block_on(async move { builder.await }))?;

        self.shard_manager = Some(client.shard_manager.clone());
        spawn(async move {
            if let Err(err) = client.start().await {
                error!(target: "discord_bot", "failed {err:?}");
            }
        });

        Ok(())
    }

    fn shutdown(&mut self) {
        if let Some(manager) = self.shard_manager.take() {
            spawn(async move {
                manager.shutdown_all().await;
            });
        }
    }
}

#[derive(Debug)]
struct DefaultEventHandler {
    command_sender: Sender<BotCommand>,
    stream_handle: Arc<Mutex<Option<JoinHandle<()>>>>,
}

#[async_trait]
impl EventHandler for DefaultEventHandler {
    async fn ready(&self, context: Context, _: Ready) {
        let commands = BotCommandKindInner::iter()
            .map(|kind| {
                let command = CreateCommand::new(kind.to_string())
                    .description(kind.get_message().expect("message already set"));
                match kind {
                    BotCommandKindInner::Chat => command.add_option(
                        CreateCommandOption::new(
                            CommandOptionType::String,
                            "message",
                            "The message to send",
                        )
                        .required(true)
                        .min_length(1),
                    ),
                    BotCommandKindInner::Stop => command.add_option(CreateCommandOption::new(
                        CommandOptionType::Boolean,
                        "go-to-town",
                        "Whether to go to town when stopping",
                    )),
                    BotCommandKindInner::StartStream
                    | BotCommandKindInner::StopStream
                    | BotCommandKindInner::Start
                    | BotCommandKindInner::Status => command,
                }
            })
            .collect::<Vec<_>>();
        if let Err(err) = Command::set_global_commands(context.http(), commands).await {
            error!(target: "discord_bot", "failed to set commands {err:?}");
        }
    }

    async fn interaction_create(&self, context: Context, interaction: Interaction) {
        if let Interaction::Command(command) = interaction {
            debug!(target: "discord_bot", "received slash command {:?}", command.data);
            if command.defer(context.http()).await.is_err() {
                return;
            }

            let kind = match command.data.name.parse::<BotCommandKindInner>() {
                Ok(kind) => kind,
                Err(_) => {
                    response_with(&context, &command, "Ignored an unknown command.").await;
                    return;
                }
            };
            match kind {
                BotCommandKindInner::StartStream => {
                    start_stream_command(self, context, command).await;
                }
                BotCommandKindInner::StopStream => {
                    stop_stream_command(self, context, command).await;
                }
                BotCommandKindInner::Start => {
                    single_command(
                        &self.command_sender,
                        &context,
                        &command,
                        BotCommandKind::Start,
                    )
                    .await;
                }
                BotCommandKindInner::Stop => {
                    single_command(
                        &self.command_sender,
                        &context,
                        &command,
                        BotCommandKind::Stop,
                    )
                    .await;
                }
                BotCommandKindInner::Status => {
                    single_command(
                        &self.command_sender,
                        &context,
                        &command,
                        BotCommandKind::Status,
                    )
                    .await;
                }
                BotCommandKindInner::Chat => {
                    single_command(
                        &self.command_sender,
                        &context,
                        &command,
                        BotCommandKind::Chat,
                    )
                    .await;
                }
            }
        }
    }
}

async fn single_command(
    sender: &Sender<BotCommand>,
    context: &Context,
    command: &CommandInteraction,
    kind: BotCommandKind,
) {
    let (tx, rx) = oneshot::channel();
    let inner = BotCommand {
        kind,
        options: command.data.options.clone(),
        sender: tx,
    };
    if sender.send(inner).await.is_err() {
        response_with(context, command, "Command failed, please try again.").await;
        return;
    }

    let builder = match timeout(Duration::from_secs(10), rx)
        .await
        .ok()
        .and_then(|inner| inner.ok())
    {
        Some(builder) => builder,
        None => {
            response_with(context, command, "Command failed, please try again.").await;
            return;
        }
    };
    let _ = command.edit_response(context.http(), builder).await;
}

async fn start_stream_command(
    handler: &DefaultEventHandler,
    context: Context,
    command: CommandInteraction,
) {
    let sender = handler.command_sender.clone();
    let mut handle = handler.stream_handle.lock().await;
    if handle.is_some() {
        response_with(&context, &command, "Streaming already started.").await;
        return;
    }

    let task = spawn(async move {
        let start_time = Instant::now();
        let max_duration = Duration::from_mins(15);
        while start_time.elapsed() < max_duration {
            single_command(&sender, &context, &command, BotCommandKind::Status).await;
            sleep(Duration::from_millis(500)).await;
        }
        response_with(&context, &command, "Streaming finished.").await;
    });

    *handle = Some(task);
}

async fn stop_stream_command(
    handler: &DefaultEventHandler,
    context: Context,
    command: CommandInteraction,
) {
    let mut stopped = false;
    if let Some(handle) = handler.stream_handle.lock().await.take()
        && !handle.is_finished()
    {
        handle.abort();
        stopped = true;
    }

    let content = if stopped {
        "Streaming stopped."
    } else {
        "No active stream to stop."
    };
    response_with(&context, &command, content).await;
}

#[inline]
async fn response_with(
    context: &Context,
    command: &CommandInteraction,
    content: impl Into<String>,
) {
    let builder = EditInteractionResponse::new().content(content);
    let _ = command.edit_response(context.http(), builder).await;
}

#[cfg(test)]
mod tests {
    // TODO
}
