use std::{sync::Arc, time::Duration};

use anyhow::Result;
use log::{debug, error};
use serenity::all::{
    CacheHttp, Command, CommandDataOption, CommandOptionType, Context, CreateCommand,
    CreateCommandOption, CreateInteractionResponse, EditInteractionResponse, EventHandler,
    GatewayIntents, Interaction, Ready, ShardManager,
};
use serenity::{Client, async_trait};
use strum::{Display, EnumIter, EnumMessage, EnumString, IntoEnumIterator};
use tokio::{
    runtime::Handle,
    spawn,
    sync::{
        mpsc::{Receiver, Sender, channel},
        oneshot,
    },
    task::block_in_place,
    time::timeout,
};

#[derive(Debug, EnumIter, EnumString, EnumMessage, Display)]
pub enum BotCommandKind {
    #[strum(to_string = "start", message = "Start the bot actions")]
    Start,
    #[strum(to_string = "stop", message = "Stop the bot actions")]
    Stop,
    #[strum(to_string = "status", message = "See bot current status")]
    Status,
    #[strum(to_string = "chat", message = "Send a message inside the game")]
    Chat,
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
}

#[async_trait]
impl EventHandler for DefaultEventHandler {
    async fn ready(&self, context: Context, _: Ready) {
        let commands = BotCommandKind::iter()
            .map(|kind| {
                let command = CreateCommand::new(kind.to_string())
                    .description(kind.get_message().expect("message already set"));
                match kind {
                    BotCommandKind::Chat => command.add_option(
                        CreateCommandOption::new(
                            CommandOptionType::String,
                            "message",
                            "The message to send",
                        )
                        .required(true)
                        .min_length(1),
                    ),
                    BotCommandKind::Stop => command.add_option(CreateCommandOption::new(
                        CommandOptionType::Boolean,
                        "go-to-town",
                        "Whether to go to town when stopping",
                    )),
                    BotCommandKind::Start | BotCommandKind::Status => command,
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
            if let Err(error) = command.defer(context.http()).await {
                error!(target: "discord_bot", "failed to defer command {error:?}");
                return;
            }

            let (tx, rx) = oneshot::channel();
            let inner = match command.data.name.parse::<BotCommandKind>() {
                Ok(kind) => BotCommand {
                    kind,
                    options: command.data.options.clone(),
                    sender: tx,
                },
                Err(_) => return,
            };
            if let Err(inner) = self.command_sender.send(inner).await {
                error!(target: "discord_bot", "failed to send command {inner:?}");
                let ack = CreateInteractionResponse::Acknowledge;
                let _ = command.create_response(context.http(), ack).await;
                return;
            }

            let builder = match timeout(Duration::from_secs(10), rx).await {
                Ok(Ok(builder)) => builder,
                _ => {
                    error!(target: "discord_bot", "failed when waiting for command response");
                    let _ = command.delete_response(context.http()).await;
                    return;
                }
            };
            if let Err(error) = command.edit_response(context.http(), builder).await {
                error!(target: "discord_bot", "failed to response to command {error:?}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    // TODO
}
