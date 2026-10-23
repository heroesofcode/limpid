//! The desktop's light/dark preference, for machines that are not Omarchy.
//!
//! `org.freedesktop.appearance` is the one piece of theming every desktop
//! agrees on. It carries no palette — only a preference between light and
//! dark — which is exactly why Limpid keeps its own colours and uses this
//! solely to choose between them.
//!
//! Omarchy sets it too, as a side effect of `omarchy-theme-set-gnome`, but it
//! does so in a block that races the theme swap. On Omarchy the palette file
//! is authoritative and this module is not consulted.

use std::sync::mpsc::{Receiver, channel};

use futures_util::StreamExt;

use crate::palette::Mode;

/// Portal service coordinates.
const DESTINATION: &str = "org.freedesktop.portal.Desktop";
const PATH: &str = "/org/freedesktop/portal/desktop";
const INTERFACE: &str = "org.freedesktop.portal.Settings";
const NAMESPACE: &str = "org.freedesktop.appearance";
const KEY: &str = "color-scheme";

/// Read the desktop's current preference.
///
/// `None` when there is no session bus, no portal, or no stated preference —
/// all of which are ordinary, and none of which are worth reporting as an
/// error.
pub fn preferred_mode() -> Option<Mode> {
    // zbus has no blocking API since 6.0; one `block_on` around the whole
    // exchange is its replacement, and this runs once, at start-up.
    let scheme = zbus::block_on(async {
        let connection = zbus::Connection::session().await.ok()?;
        let reply = connection
            .call_method(
                Some(DESTINATION),
                PATH,
                Some(INTERFACE),
                "ReadOne",
                &(NAMESPACE, KEY),
            )
            .await
            .ok()?;
        let value: zbus::OwnedVariant = reply.body().deserialize().ok()?;
        u32::try_from(&value).ok()
    })?;
    mode_from_scheme(scheme)
}

/// Map the portal's enumeration onto a mode.
///
/// 0 is "no preference", which is not the same as dark and should leave the
/// choice alone.
fn mode_from_scheme(scheme: u32) -> Option<Mode> {
    match scheme {
        1 => Some(Mode::Dark),
        2 => Some(Mode::Light),
        _ => None,
    }
}

/// A handle yielding the desktop's preference each time it changes.
pub struct Watcher {
    updates: Receiver<Mode>,
}

impl Watcher {
    /// The most recent preference, if one has arrived.
    pub fn try_next(&self) -> Option<Mode> {
        let mut latest = None;
        while let Ok(mode) = self.updates.try_recv() {
            latest = Some(mode);
        }
        latest
    }
}

/// Watch the appearance setting.
///
/// Fails when there is no session bus to watch, which is the case in CI and
/// on a headless machine.
pub fn watch() -> zbus::Result<Watcher> {
    let (tx, rx) = channel();

    // Subscribe before the thread starts, so that a missing session bus is
    // reported to the caller rather than ending a thread nobody watches.
    let mut stream = zbus::block_on(async {
        let connection = zbus::Connection::session().await?;
        let rule = zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .interface(INTERFACE)
            .member("SettingChanged")
            .build()?;
        zbus::MessageStream::for_match_rule(rule, &connection, None).await
    })?;

    std::thread::Builder::new()
        .name("limpid-appearance-watch".to_owned())
        .spawn(move || {
            zbus::block_on(async move {
                while let Some(message) = stream.next().await {
                    let Ok(message) = message else {
                        continue;
                    };
                    // The body has to outlive the deserialised value, which
                    // borrows from it.
                    let body = message.body();
                    let Ok((namespace, key, value)) =
                        body.deserialize::<(String, String, zbus::Variant<'_>)>()
                    else {
                        continue;
                    };
                    if namespace != NAMESPACE || key != KEY {
                        continue;
                    }
                    let Ok(scheme) = u32::try_from(&value) else {
                        continue;
                    };
                    if let Some(mode) = mode_from_scheme(scheme)
                        && tx.send(mode).is_err()
                    {
                        break;
                    }
                }
            });
        })
        .map_err(|error| zbus::Error::InputOutput(error.into()))?;

    Ok(Watcher { updates: rx })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_portal_enumeration_maps_onto_a_mode() {
        assert_eq!(mode_from_scheme(1), Some(Mode::Dark));
        assert_eq!(mode_from_scheme(2), Some(Mode::Light));
    }

    #[test]
    fn no_preference_leaves_the_choice_alone() {
        // 0 means the desktop has no opinion, which must not be read as dark.
        assert_eq!(mode_from_scheme(0), None);
        assert_eq!(mode_from_scheme(99), None);
    }

    #[test]
    fn reading_without_a_session_bus_is_not_an_error() {
        // Runs in CI, where there is no bus; the point is that it returns
        // rather than panicking or blocking.
        let _ = preferred_mode();
    }
}
