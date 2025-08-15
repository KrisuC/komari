use std::{
    cell::RefCell,
    mem,
    ops::{Index, Not},
    rc::Rc,
    sync::{Arc, Mutex},
    time::Duration,
};

use anyhow::{Error, Ok, bail};
use bit_vec::BitVec;
use log::{debug, error};
use reqwest::Url;
use serenity::all::{CreateAttachment, ExecuteWebhook, Http, Webhook};
use tokio::{
    spawn,
    time::{Instant, sleep},
};

use crate::Settings;

static TRUE: bool = true;
static FALSE: bool = false;

#[derive(PartialEq, Eq, Clone, Copy, Debug)]
#[repr(usize)]
pub enum NotificationKind {
    FailOrMapChange,
    RuneAppear,
    EliteBossAppear,
    PlayerGuildieAppear,
    PlayerStrangerAppear,
    PlayerFriendAppear,
    PlayerIsDead,
}

impl From<NotificationKind> for usize {
    fn from(kind: NotificationKind) -> Self {
        kind as usize
    }
}

impl Index<NotificationKind> for BitVec {
    type Output = bool;

    fn index(&self, index: NotificationKind) -> &Self::Output {
        if self.get(index.into()).expect("index out of bound") {
            &TRUE
        } else {
            &FALSE
        }
    }
}

/// A notification scheduled to be sending.
#[derive(Debug)]
struct ScheduledNotification {
    /// The instant it was scheduled.
    instant: Instant,
    /// The kind of notification.
    kind: NotificationKind,
    /// The webhook url.
    url: String,
    /// The content of the message.
    content: String,
    /// The username of the message's owner.
    username: &'static str,
    /// Stores fixed size tuples of frame and frame deadline in seconds.
    ///
    /// During each [`DiscordNotification::update_schedule`], the first frame not passing the
    /// deadline will try to capture the image from current game state. This is useful for showing
    /// `before and after` when map changes. So frame that cannot capture when the deadline is
    /// reached will be skipped.
    frames: Vec<(Option<Vec<u8>>, u32)>,
}

#[derive(Debug)]
pub struct DiscordNotification {
    /// A reference to [`Settings`] for checking if a notification is enabled.
    settings: Rc<RefCell<Settings>>,
    /// Stores pending notifications.
    scheduled: Arc<Mutex<Vec<ScheduledNotification>>>,
    /// Storing currently incomplete / pending notification kinds.
    ///
    /// There can only be one unique [`NotificationKind`] scheduled at a time.
    pending: Arc<Mutex<BitVec>>,
}

impl DiscordNotification {
    pub fn new(settings: Rc<RefCell<Settings>>) -> Self {
        Self {
            settings,
            scheduled: Arc::new(Mutex::new(vec![])),
            pending: Arc::new(Mutex::new(BitVec::from_elem(
                mem::variant_count::<NotificationKind>(),
                false,
            ))),
        }
    }

    pub fn schedule_notification(&self, kind: NotificationKind) -> Result<(), Error> {
        let settings = self.settings.borrow();
        let is_enabled = match kind {
            NotificationKind::FailOrMapChange => {
                settings.notifications.notify_on_fail_or_change_map
            }
            NotificationKind::RuneAppear => settings.notifications.notify_on_rune_appear,
            NotificationKind::EliteBossAppear => settings.notifications.notify_on_elite_boss_appear,
            NotificationKind::PlayerIsDead => settings.notifications.notify_on_player_die,
            NotificationKind::PlayerGuildieAppear => {
                settings.notifications.notify_on_player_guildie_appear
            }
            NotificationKind::PlayerStrangerAppear => {
                settings.notifications.notify_on_player_stranger_appear
            }
            NotificationKind::PlayerFriendAppear => {
                settings.notifications.notify_on_player_friend_appear
            }
        };
        if !is_enabled {
            bail!("notification not enabled");
        }
        if settings.notifications.discord_webhook_url.is_empty() {
            bail!("webhook url not provided");
        }

        let mut pending = self.pending.lock().unwrap();
        if pending[kind] {
            bail!("notification is already sending");
        }

        let url = settings.notifications.discord_webhook_url.clone();
        if Url::try_from(url.as_str()).is_err() {
            bail!("failed to parse webhook url");
        }

        let user_id = settings
            .notifications
            .discord_user_id
            .is_empty()
            .not()
            .then_some(format!("<@{}> ", settings.notifications.discord_user_id))
            .unwrap_or_default();
        let content = match kind {
            NotificationKind::FailOrMapChange => {
                if self.settings.borrow().stop_on_fail_or_change_map {
                    format!(
                        "{user_id}Bot stopped because it has failed to detect or the map has changed"
                    )
                } else {
                    format!("{user_id}Bot has failed to detect or the map has changed")
                }
            }
            NotificationKind::RuneAppear => {
                format!("{user_id}Bot has detected a rune on map")
            }
            NotificationKind::EliteBossAppear => {
                format!("{user_id}Elite boss spawned")
            }
            NotificationKind::PlayerIsDead => {
                format!("{user_id}The player is dead")
            }
            NotificationKind::PlayerGuildieAppear => {
                format!("{user_id}Bot has detected guildie player(s)")
            }
            NotificationKind::PlayerStrangerAppear => {
                format!("{user_id}Bot has detected stranger player(s)")
            }
            NotificationKind::PlayerFriendAppear => {
                format!("{user_id}Bot has detected friend player(s)")
            }
        };
        let frames = match kind {
            NotificationKind::FailOrMapChange => vec![(None, 2), (None, 4)],
            NotificationKind::EliteBossAppear
            | NotificationKind::PlayerIsDead
            | NotificationKind::PlayerGuildieAppear
            | NotificationKind::PlayerStrangerAppear
            | NotificationKind::PlayerFriendAppear
            | NotificationKind::RuneAppear => vec![(None, 2)],
        };
        let delay = match kind {
            NotificationKind::FailOrMapChange => 5,
            NotificationKind::EliteBossAppear
            | NotificationKind::PlayerIsDead
            | NotificationKind::PlayerGuildieAppear
            | NotificationKind::PlayerStrangerAppear
            | NotificationKind::PlayerFriendAppear
            | NotificationKind::RuneAppear => 3,
        };

        let mut scheduled = self.scheduled.lock().unwrap();
        scheduled.push(ScheduledNotification {
            instant: Instant::now(),
            kind,
            url,
            content,
            username: "maple-bot",
            frames,
        });
        pending.set(kind.into(), true);

        let pending = self.pending.clone();
        let scheduled = self.scheduled.clone();
        spawn(async move {
            sleep(Duration::from_secs(delay)).await;

            let notification = scheduled
                .lock()
                .ok()
                .map(|mut scheduled| {
                    // Inside closure or compiler will complain about MutexGuard not being Send
                    let (index, _) = scheduled
                        .iter()
                        .enumerate()
                        .find(|(_, item)| item.kind == kind)
                        .unwrap();
                    scheduled.remove(index)
                })
                .unwrap();
            let kind = notification.kind;
            debug_assert!(
                pending
                    .lock()
                    .unwrap()
                    .get(notification.kind.into())
                    .unwrap()
            );
            pending.lock().unwrap().set(kind.into(), false);
            let _ = post_notification(notification).await;
        });

        Ok(())
    }

    pub fn update(&self, frame: impl Fn() -> Option<Vec<u8>>) {
        let mut scheduled = self.scheduled.lock().unwrap();
        if scheduled.is_empty() {
            return;
        }

        for item in scheduled.iter_mut() {
            let elapsed_secs = item.instant.elapsed().as_secs() as u32;
            for (item_frame, deadline) in item.frames.iter_mut() {
                if elapsed_secs <= *deadline {
                    if item_frame.is_none() {
                        *item_frame = frame();
                    }
                    break;
                }
            }
        }
    }
}

async fn post_notification(notification: ScheduledNotification) -> Result<(), Error> {
    let http = Http::new("");
    let webhook = Webhook::from_url(&http, &notification.url).await?;
    let files = notification
        .frames
        .into_iter()
        .filter_map(|(frame, _)| frame)
        .enumerate()
        .map(|(index, frame)| {
            CreateAttachment::bytes(frame, format!("image_{index}.png"))
                .description(format!("Game snapshot #{index}"))
        });

    let builder = ExecuteWebhook::new()
        .content(notification.content)
        .username(notification.username)
        .files(files);
    let _ = webhook
        .execute(&http, false, builder)
        .await
        .inspect(|_| {
            debug!(target: "notification", "calling Webhook API {:?} succeeded", notification.kind);
        })
        .inspect_err(|err| {
            error!(target: "notification", "calling Webhook API failed {err}");
        });

    Ok(())
}

#[cfg(test)]
mod test {
    use std::{cell::RefCell, rc::Rc, time::Duration};

    use tokio::time::{Instant, advance};

    use super::{DiscordNotification, NotificationKind, ScheduledNotification};
    use crate::{Notifications, Settings};

    #[tokio::test(start_paused = true)]
    async fn schedule_kind_unique() {
        let noti = DiscordNotification::new(Rc::new(RefCell::new(Settings {
            notifications: Notifications {
                discord_webhook_url: "https://discord.com/api/webhooks/foo/bar".to_string(),
                notify_on_fail_or_change_map: true,
                notify_on_rune_appear: true,
                ..Default::default()
            },
            ..Default::default()
        })));

        assert!(
            noti.schedule_notification(NotificationKind::FailOrMapChange)
                .is_ok()
        );
        assert!(noti.scheduled.lock().unwrap().len() == 1);
        assert!(
            noti.pending
                .lock()
                .unwrap()
                .get(NotificationKind::FailOrMapChange.into())
                .unwrap()
        );
        assert!(
            noti.schedule_notification(NotificationKind::FailOrMapChange)
                .is_err()
        );
        assert!(
            noti.schedule_notification(NotificationKind::RuneAppear)
                .is_ok()
        );
    }

    #[tokio::test(start_paused = true)]
    async fn schedule_invalid_url() {
        let noti = DiscordNotification::new(Rc::new(RefCell::new(Settings {
            notifications: Notifications {
                notify_on_fail_or_change_map: true,
                ..Default::default()
            },
            ..Default::default()
        })));

        assert!(
            noti.schedule_notification(NotificationKind::FailOrMapChange)
                .is_err()
        );
    }

    #[tokio::test(start_paused = true)]
    #[allow(clippy::await_holding_lock)]
    async fn update_scheduled_frames_deadline() {
        let noti = DiscordNotification::new(Rc::new(RefCell::new(Settings::default())));
        noti.scheduled.lock().unwrap().push(ScheduledNotification {
            instant: Instant::now(),
            kind: NotificationKind::FailOrMapChange,
            url: "https://example.com".into(),
            content: "content".into(),
            username: "username",
            frames: vec![(None, 3), (None, 6), (None, 9)],
        });

        advance(Duration::from_secs(4)).await;
        // Skip frame 1 because deadline passed to frame 2
        noti.update(|| Some(vec![]));
        let scheduled_guard = noti.scheduled.lock().unwrap();
        let scheduled = scheduled_guard.first().unwrap();
        assert!(scheduled.frames[0].0.is_none());
        assert!(scheduled.frames[1].0.is_some());
        assert!(scheduled.frames[2].0.is_none());
        drop(scheduled_guard);

        // Frame 3
        advance(Duration::from_secs(4)).await;
        noti.update(|| Some(vec![]));
        let scheduled = noti.scheduled.lock().unwrap();
        let scheduled = scheduled.first().unwrap();
        assert!(scheduled.frames[0].0.is_none());
        assert!(scheduled.frames[1].0.is_some());
        assert!(scheduled.frames[2].0.is_some());
    }
}
