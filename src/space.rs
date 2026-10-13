//! Spaces: what the expanded notch presents. Phase 4.1 is state only: every
//! space renders the existing UI. The active space lives in `WindowState` and is
//! never persisted (each launch starts in `Home`). All spaces share the one
//! `MediaEngine` (session, metadata, artwork, accent, timeline, visualizer).

/// The active space. Adding a variant is a compile error at every exhaustive
/// `match`, so an unknown space cannot appear by accident.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OecaSpace {
    #[default]
    Home,
    Music,
    /// The in-memory clipboard history (Phase 5.2).
    Clipboard,
}

/// What the expanded notch shows: a space, or the drop page while an image is
/// dragged over it. Animations blend between two scenes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scene {
    Space(OecaSpace),
    /// The Settings page, over the active space (which stays selected).
    Settings,
    Drop,
}

/// The one panel size: the drop page and Settings always take the Clipboard
/// space's settled size, whichever space is active (so they cannot drift).
pub const PANEL_SPACE: OecaSpace = OecaSpace::Clipboard;

/// In-memory user settings (every launch starts from the defaults).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OecaSettings {
    /// Topmost band (above normal windows). On by default: Oeca has always
    /// been a topmost window.
    pub always_on_top: bool,
    /// Snap instead of animating (the same snap path as the system's
    /// reduced-motion preference). Off by default.
    pub reduced_motion: bool,
    /// Escape closes the full Settings window. On by default (the window's
    /// original behavior); its title-bar X always works.
    pub close_settings_on_escape: bool,
    /// The Clipboard space's Clear asks before forgetting the history. On by
    /// default.
    pub confirm_clear_clipboard: bool,
    /// Accent color (Settings > Appearance). White by default.
    pub accent: crate::config::AccentChoice,
    /// Entries the clipboard history keeps (Settings > Clipboard). 20 by
    /// default.
    pub clipboard_capacity: crate::config::ClipboardCapacity,
    /// The source-app badge on the Home artwork (Settings > Media). On by
    /// default.
    pub show_source_app: bool,
    /// The Music space's playback visualizer (Settings > Media). On by
    /// default; the collapsed notch keeps its own.
    pub show_visualizer: bool,
    /// How long after an outside click the expanded notch collapses
    /// (Settings > General). 5 s by default.
    pub collapse_delay: crate::config::CollapseDelay,
}

impl Default for OecaSettings {
    fn default() -> Self {
        Self {
            always_on_top: true,
            reduced_motion: false,
            close_settings_on_escape: true,
            confirm_clear_clipboard: true,
            accent: crate::config::AccentChoice::White,
            clipboard_capacity: crate::config::ClipboardCapacity::Twenty,
            show_source_app: true,
            show_visualizer: true,
            collapse_delay: crate::config::CollapseDelay::Five,
        }
    }
}

impl Scene {
    /// The space whose settled size this scene takes: its own, or the panel
    /// size for Settings and the drop page.
    pub fn size_space(self) -> OecaSpace {
        match self {
            Self::Space(space) => space,
            Self::Settings | Self::Drop => PANEL_SPACE,
        }
    }

    /// What the expanded notch shows: an image drag's drop page first, then
    /// an open Settings page, otherwise the active space.
    pub fn current(drop_page: bool, settings_open: bool, space: OecaSpace) -> Self {
        if drop_page {
            Self::Drop
        } else if settings_open {
            Self::Settings
        } else {
            Self::Space(space)
        }
    }
}

impl OecaSpace {
    #[allow(dead_code)]
    pub const ALL: [Self; 3] = [Self::Home, Self::Music, Self::Clipboard];

    #[allow(dead_code)]
    pub fn is_home(self) -> bool {
        self == Self::Home
    }

    #[allow(dead_code)]
    pub fn is_music(self) -> bool {
        self == Self::Music
    }

    /// Selector label / accessible name.
    pub fn label(self) -> &'static str {
        match self {
            Self::Home => "Home",
            Self::Music => "Music",
            Self::Clipboard => "Clipboard",
        }
    }

    /// Switches to `space`; returns true if the active space changed. The single
    /// transition point (the space selector calls this via `WindowState`).
    pub fn switch_to(&mut self, space: Self) -> bool {
        std::mem::replace(self, space) != space
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::{MediaContent, PlaybackState, Visualizer};

    #[test]
    fn test_default_is_home_and_spaces_are_distinct() {
        let s = OecaSpace::default();
        assert_eq!(s, OecaSpace::Home);
        assert!(s.is_home() && !s.is_music());
        assert_ne!(OecaSpace::Home, OecaSpace::Music);
        assert!(OecaSpace::Music.is_music() && !OecaSpace::Music.is_home());
    }

    #[test]
    fn test_switching_home_music_and_back() {
        let mut s = OecaSpace::default();
        assert!(s.switch_to(OecaSpace::Music), "Home -> Music");
        assert!(s.is_music());
        assert!(!s.switch_to(OecaSpace::Music), "same space is not a change");
        assert!(s.switch_to(OecaSpace::Home), "Music -> Home");
        assert!(s.is_home());
        for i in 0..100 {
            let target = OecaSpace::ALL[i % 3];
            s.switch_to(target);
            assert_eq!(s, target, "repeated switching");
        }
    }

    #[test]
    fn test_settings_and_drop_take_the_clipboard_size_from_any_space() {
        for space in OecaSpace::ALL {
            assert_eq!(
                Scene::current(false, true, space).size_space(),
                OecaSpace::Clipboard
            );
            assert_eq!(
                Scene::current(true, false, space).size_space(),
                OecaSpace::Clipboard
            );
            assert_eq!(Scene::current(false, false, space).size_space(), space);
        }
        let s = OecaSettings::default();
        assert!(s.always_on_top && !s.reduced_motion);
        assert!(s.close_settings_on_escape && s.confirm_clear_clipboard);
    }

    #[test]
    fn test_scene_priority_and_settings_keep_the_space() {
        for space in OecaSpace::ALL {
            assert_eq!(Scene::current(false, false, space), Scene::Space(space));
            assert_eq!(Scene::current(false, true, space), Scene::Settings);
            assert_eq!(Scene::current(true, true, space), Scene::Drop, "drag wins");
            // Closing Settings returns to the space that was active
            assert_eq!(Scene::current(false, false, space), Scene::Space(space));
        }
    }

    #[test]
    fn test_labels() {
        assert_eq!(
            OecaSpace::ALL.map(OecaSpace::label),
            ["Home", "Music", "Clipboard"]
        );
    }

    #[test]
    fn test_exactly_three_spaces() {
        // Exhaustive match (no wildcard): a fourth variant fails to compile here
        let index = |s: OecaSpace| match s {
            OecaSpace::Home => 0,
            OecaSpace::Music => 1,
            OecaSpace::Clipboard => 2,
        };
        assert_eq!(OecaSpace::ALL.len(), 3);
        assert_eq!(OecaSpace::ALL.map(index), [0, 1, 2]);
    }

    #[test]
    fn test_home_clipboard_music_round_trip() {
        let mut s = OecaSpace::default();
        assert!(s.switch_to(OecaSpace::Clipboard));
        assert!(s.switch_to(OecaSpace::Music));
        assert!(s.switch_to(OecaSpace::Clipboard));
        assert!(s.switch_to(OecaSpace::Home));
        assert!(s.is_home());
    }

    #[test]
    fn test_switching_leaves_shared_media_state_alone() {
        // Spaces hold no media data: the shared model is identical before/after
        let content = MediaContent {
            title: "Song".into(),
            playback: PlaybackState::Playing,
            accent: Some([10, 20, 30]),
            ..MediaContent::test_default()
        };
        let mut viz = Visualizer::default();
        viz.set_playing(true);
        viz.step(100.0);
        let (before_content, before_viz) = (content.clone(), viz);
        let mut s = OecaSpace::default();
        s.switch_to(OecaSpace::Music);
        s.switch_to(OecaSpace::Home);
        s.switch_to(OecaSpace::Music);
        assert_eq!(content, before_content);
        assert_eq!(viz, before_viz);
        // ...and media updates don't touch the space
        let mut content = content;
        content.title = "Next".into();
        content.playback = PlaybackState::Paused;
        assert!(s.is_music(), "space survives media updates");
    }

    #[test]
    fn test_no_persistence_each_start_is_home() {
        // A fresh state (as at every launch) is always Home, whatever was active
        let mut previous = OecaSpace::default();
        previous.switch_to(OecaSpace::Music);
        assert_eq!(OecaSpace::default(), OecaSpace::Home);
    }
}
