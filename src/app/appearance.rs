//! Appearance preferences and the Kit settings page.
use super::FerryDock;
use gpui::{App, Context, Div, FontWeight, Global, Window, div, prelude::*, px, rgb};
use gpui_kit::component::{
    ActiveTheme, Selectable, Sizable, Theme, ThemeMode,
    alert::Alert,
    button::Button,
    setting::{SettingField, SettingGroup, SettingItem, SettingPage, Settings},
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(super) enum AppTheme {
    #[default]
    FerryDock,
    Dark,
    Light,
}

impl AppTheme {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::FerryDock => "FerryDock",
            Self::Dark => "Dark",
            Self::Light => "Light",
        }
    }

    fn id(self) -> &'static str {
        match self {
            Self::FerryDock => "theme-ferrydock",
            Self::Dark => "theme-dark",
            Self::Light => "theme-light",
        }
    }

    pub(super) fn apply(self, cx: &mut App) {
        // Reload Kit's base theme before applying the custom palette so switching
        // back to standard Dark/Light cannot inherit FerryDock's overrides.
        Theme::change(
            if self == Self::Light {
                ThemeMode::Light
            } else {
                ThemeMode::Dark
            },
            None,
            cx,
        );
        if self == Self::FerryDock {
            Theme::update(cx, |theme| {
                theme.colors.background = rgb(0x171d21).into();
                theme.colors.foreground = rgb(0xe0ece7).into();
                theme.colors.muted_foreground = rgb(0x9eb2a8).into();
                theme.colors.border = rgb(0x34433c).into();
                theme.colors.primary = rgb(0x74d7b1).into();
                theme.colors.ring = rgb(0x74d7b1).into();
                theme.colors.selection = rgb(0x234136).into();
                theme.colors.sidebar_accent = rgb(0x234136).into();
                theme.colors.sidebar_accent_foreground = rgb(0x74d7b1).into();
                theme.colors.list_active = rgb(0x234136).into();
                theme.colors.status_bar = rgb(0x171d21).into();
                theme.colors.status_bar_border = rgb(0x34433c).into();
            });
        }
    }
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct SavedAppearance {
    theme: AppTheme,
}

pub(super) struct Appearance {
    pub(super) theme: AppTheme,
    path: Option<PathBuf>,
    error: Option<String>,
}
impl Global for Appearance {}

fn load(path: &Path) -> Result<AppTheme, String> {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice::<SavedAppearance>(&bytes)
            .map(|saved| saved.theme)
            .map_err(|error| format!("Cannot read appearance settings: {error}")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(AppTheme::default()),
        Err(error) => Err(format!("Cannot read appearance settings: {error}")),
    }
}

fn save(path: &Path, theme: AppTheme) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or("Appearance settings need a directory")?;
    std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let bytes =
        serde_json::to_vec_pretty(&SavedAppearance { theme }).map_err(|error| error.to_string())?;
    let mut temporary =
        tempfile::NamedTempFile::new_in(parent).map_err(|error| error.to_string())?;
    std::io::Write::write_all(&mut temporary, &bytes).map_err(|error| error.to_string())?;
    temporary
        .persist(path)
        .map(|_| ())
        .map_err(|error| error.error.to_string())
}

pub(super) fn init(cx: &mut App) {
    init_path(
        crate::compose::Settings::path()
            .map(|path| path.with_file_name("appearance.json"))
            .map_err(|error| error.to_string()),
        cx,
    );
}

#[cfg(all(test, feature = "headless-tests"))]
pub(super) fn init_test(path: PathBuf, cx: &mut App) {
    init_path(Ok(path), cx);
}

fn init_path(path: Result<PathBuf, String>, cx: &mut App) {
    let (path, theme, error) = match path {
        Ok(path) => match load(&path) {
            Ok(theme) => (Some(path), theme, None),
            Err(error) => (Some(path), AppTheme::default(), Some(error)),
        },
        Err(error) => (None, AppTheme::default(), Some(error.to_string())),
    };
    cx.set_global(Appearance { theme, path, error });
    theme.apply(cx);
}

fn choose(theme: AppTheme, cx: &mut App) {
    let error = match cx.global::<Appearance>().path.as_deref() {
        Some(path) => save(path, theme).err(),
        None => Some("The configuration directory is unavailable".into()),
    };
    cx.update_global::<Appearance, _>(|appearance, _| {
        appearance.theme = theme;
        appearance.error = error.map(|error| {
            format!("The theme changed for this session, but could not be saved: {error}")
        });
    });
    theme.apply(cx);
}

impl FerryDock {
    pub(super) fn show_settings(
        &mut self,
        open: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.settings_open = open;
        if !open && (self.images_open || self.volumes_open || self.networks_open) {
            self.focus.focus(window, cx);
        } else if !open && let Some(form) = &self.projects.form {
            use gpui::Focusable;
            form.directory.focus_handle(cx).focus(window, cx);
        } else if !open
            && self.tab == super::Tab::Terminal
            && let Some(terminal) = &self.terminal.view
        {
            terminal.update(cx, |view, cx| view.focus(window, cx));
        } else if !open && self.tab == super::Tab::Inspect {
            use gpui::Focusable;
            self.containers
                .inspection
                .focus_handle(cx)
                .focus(window, cx);
        } else {
            self.focus.focus(window, cx);
        }
        cx.notify();
    }

    pub(super) fn appearance_settings(&self, cx: &mut Context<Self>) -> Div {
        div().size_full().flex().flex_col().gap_4().p_6()
            .child(div().text_size(px(22.0)).font_weight(FontWeight::MEDIUM).child("Settings"))
            .when_some(cx.global::<Appearance>().error.clone(), |view, error| view.child(Alert::error("appearance-error", error)))
            .child(Settings::new("appearance-settings").pages(vec![
                SettingPage::new("Appearance").default_open(true).group(
                    SettingGroup::new().title("Theme").description("Choose the appearance of FerryDock. Your choice is saved on this computer.")
                        .item(SettingItem::new("Color theme", SettingField::render(|_, _, cx| {
                            let selected = cx.global::<Appearance>().theme;
                            div().flex().flex_wrap().gap_2().children(
                                [AppTheme::FerryDock, AppTheme::Dark, AppTheme::Light].map(|theme| {
                                    Button::new(theme.id()).label(theme.label()).outline().selected(selected == theme)
                                        .on_click(move |_, _, cx| choose(theme, cx))
                                })
                            )
                        })).description("FerryDock keeps the original charcoal and mint palette. Dark uses neutral dark colors; Light uses bright surfaces."))
                        .item(SettingItem::new("Restore default", SettingField::render(|_, _, _| {
                            Button::new("theme-reset").small().label("Use FerryDock theme").on_click(|_, _, cx| choose(AppTheme::FerryDock, cx))
                        })))
                )
            ]).map(|settings| {
                #[cfg(target_os = "linux")]
                let settings = settings.page(self.update_settings(cx));
                settings
            }))
            .child(div().text_color(cx.theme().muted_foreground).text_sm().child("Theme changes apply immediately. Your current workspace stays open while you adjust settings."))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appearance_defaults_and_round_trips_without_touching_compose_settings() {
        let directory =
            std::env::temp_dir().join(format!("ferrydock-appearance-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("appearance.json");
        assert_eq!(
            load(&directory.join("missing.json")).unwrap(),
            AppTheme::FerryDock
        );
        std::fs::write(directory.join("compose.json"), "compose fixture").unwrap();
        for theme in [AppTheme::Light, AppTheme::Dark, AppTheme::FerryDock] {
            save(&path, theme).unwrap();
            assert_eq!(load(&path).unwrap(), theme);
        }
        assert_eq!(
            std::fs::read_to_string(directory.join("compose.json")).unwrap(),
            "compose fixture"
        );
        std::fs::write(&path, "{invalid}").unwrap();
        assert!(load(&path).is_err());
        std::fs::remove_dir_all(directory).unwrap();
    }
}
