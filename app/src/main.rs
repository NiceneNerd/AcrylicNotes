// SPDX-License-Identifier: MIT
//! M0 glass spike.
//!
//! One window with two surfaces: a translucent "chrome" pane (sidebar) and a
//! more opaque "note" pane, so the glass effect and the legibility of body
//! text can be judged side by side. Buttons switch the requested backdrop
//! style; the status line reports what the platform actually applied.
//!
//! This is a spike, not the real app: it proves the glass plumbing works
//! before the vault code is written.

use cosmic::app::Core;
use cosmic::iced::{Background, Color, Length};
use cosmic::widget;
use cosmic::Element;

use vault_glass::{Backdrop, Capabilities, GlassStyle};

fn main() -> cosmic::iced::Result {
    let settings = cosmic::app::Settings::default()
        .size(cosmic::iced::Size::new(940.0, 640.0))
        // The window itself must be transparent for any backdrop to show.
        .transparent(true);

    cosmic::app::run::<App>(settings, ())
}

struct App {
    core: Core,
    capabilities: Capabilities,
    requested: GlassStyle,
    state: Option<BackdropState>,
    error: Option<String>,
}

/// A local platform call reported what it applied.
#[derive(Clone, Copy, Debug, PartialEq)]
enum BackdropState {
    /// `vault-glass` applied the effect through a platform API.
    Applied(GlassStyle),
    /// The request was delegated (Wayland: the compositor blurs through
    /// `ext-background-effect-v1`); nothing local exists to confirm it with.
    Delegated(GlassStyle),
}

#[derive(Clone, Debug)]
enum Message {
    Request(GlassStyle),
    Applied(GlassStyle),
    Delegated(GlassStyle),
    Failed(String),
}

impl App {
    /// Ask the platform for a backdrop and report back what it managed.
    fn request_backdrop(&self, style: GlassStyle) -> cosmic::app::Task<Message> {
        let id = cosmic::iced::window::Id::RESERVED;

        #[cfg(target_os = "linux")]
        {
            // Wayland: the blur is compositor-managed. libcosmic's toolkit
            // request is the whole story and no per-window native call exists,
            // so reporting "applied" would claim a confirmation we do not
            // have. We report the request as delegated instead.
            if style != GlassStyle::Opaque && self.capabilities.preferred == GlassStyle::Blur {
                let blur = cosmic::iced::window::enable_blur::<cosmic::Action<Message>>(id);
                let note = cosmic::iced::Task::perform(std::future::ready(()), move |()| {
                    cosmic::Action::App(Message::Delegated(style))
                });
                return cosmic::app::Task::batch([blur, note]);
            }
            if style == GlassStyle::Opaque {
                let unblur = cosmic::iced::window::disable_blur::<cosmic::Action<Message>>(id);
                let note = cosmic::iced::Task::perform(std::future::ready(()), move |()| {
                    cosmic::Action::App(Message::Applied(GlassStyle::Opaque))
                });
                return cosmic::app::Task::batch([unblur, note]);
            }
            // No compositor blur and no local API: say so instead of pretending.
            return cosmic::iced::Task::perform(std::future::ready(()), move |()| {
                cosmic::Action::App(Message::Failed(
                    "no compositor blur and no local backdrop API on this platform".into(),
                ))
            })
            .into();
        }

        #[cfg(not(target_os = "linux"))]
        cosmic::iced::window::run_with_handle(id, move |handle| {
            let backdrop = vault_glass::platform::native();
            match Backdrop::apply(&*backdrop, &handle, style) {
                Ok(applied) => Message::Applied(applied),
                Err(error) => Message::Failed(error.to_string()),
            }
        })
        .map(cosmic::Action::App)
    }

    /// Every style worth offering in the UI, without duplicates.
    fn styles(&self) -> Vec<GlassStyle> {
        let mut styles = Vec::new();
        for style in std::iter::once(self.requested)
            .chain(std::iter::once(self.capabilities.preferred))
            .chain(self.capabilities.fallbacks.iter().copied())
            .chain(std::iter::once(GlassStyle::Opaque))
        {
            if !styles.contains(&style) {
                styles.push(style);
            }
        }
        styles
    }

    /// Whether the window should currently show a blurred backdrop.
    fn is_glassy(&self) -> bool {
        let state_glassy = !matches!(self.state, Some(BackdropState::Applied(GlassStyle::Opaque)));
        self.requested != GlassStyle::Opaque && state_glassy
    }
}

impl cosmic::Application for App {
    type Executor = cosmic::executor::Default;
    type Flags = ();
    type Message = Message;

    const APP_ID: &'static str = "com.example.acrylicnotes";

    fn core(&self) -> &Core {
        &self.core
    }

    fn core_mut(&mut self) -> &mut Core {
        &mut self.core
    }

    fn init(core: Core, _flags: Self::Flags) -> (Self, cosmic::app::Task<Self::Message>) {
        let capabilities = vault_glass::platform::native().probe();
        let mut app = App {
            core,
            capabilities: capabilities.clone(),
            requested: capabilities.preferred,
            state: None,
            error: None,
        };
        app.core.set_header_title("Acrylic Notes".to_string());

        let task = app.request_backdrop(app.requested);

        (app, task)
    }

    fn update(&mut self, message: Self::Message) -> cosmic::app::Task<Self::Message> {
        match message {
            Message::Request(style) => {
                self.requested = style;
                self.error = None;
                self.state = None;
                self.request_backdrop(style)
            }
            Message::Applied(style) => {
                self.state = Some(BackdropState::Applied(style));
                cosmic::app::Task::none()
            }
            Message::Delegated(style) => {
                self.state = Some(BackdropState::Delegated(style));
                cosmic::app::Task::none()
            }
            Message::Failed(error) => {
                self.error = Some(error);
                cosmic::app::Task::none()
            }
        }
    }

    fn view(&self) -> Element<'_, Self::Message> {
        let theme = cosmic::theme::active();
        let text_color = Color::from(theme.cosmic().background(theme.transparent).on);
        let glassy = self.is_glassy();

        // Chrome is meant to be see-through; the note surface stays readable.
        let chrome_alpha = if glassy { 0.55 } else { 1.0 };
        let note_alpha = if glassy { 0.93 } else { 1.0 };

        let sidebar = widget::container(
            widget::column::with_capacity(10)
                .push(widget::text::heading("Acrylic Notes"))
                .push(widget::text::caption("Journal"))
                .push(widget::text::body("2026-10-31"))
                .push(widget::text::body("2026-10-30"))
                .push(widget::text::body("2026-10-29"))
                .push(widget::text::caption("Notes"))
                .push(widget::text::body("Glass spike"))
                .push(widget::text::body("Rendering notes"))
                .push(widget::text::body("Reading list")),
        )
        .padding(16)
        .width(Length::Shrink)
        .style(move |_theme| cosmic::widget::container::Style {
            background: Some(Background::Color(Color::from_rgba(
                0.09,
                0.10,
                0.12,
                chrome_alpha,
            ))),
            text_color: Some(text_color),
            ..cosmic::widget::container::Style::default()
        });

        let mut buttons = widget::row::with_capacity(8).spacing(8);
        for style in self.styles() {
            let label = style.name();
            let button = if style == self.requested {
                widget::button::suggested(label)
            } else {
                widget::button::standard(label)
            };
            buttons = buttons.push(button.on_press(Message::Request(style)));
        }

        let status = match (&self.state, &self.error) {
            (Some(BackdropState::Delegated(style)), None) => format!(
                "requested: {} — delegated to compositor (ext-background-effect-v1): not confirmed locally",
                style.name(),
            ),
            (Some(BackdropState::Applied(applied)), None) => format!(
                "requested: {} — applied: {} — platform prefers: {}",
                self.requested.name(),
                applied.name(),
                self.capabilities.preferred.name(),
            ),
            (Some(BackdropState::Applied(applied)), Some(error)) => {
                format!("applied: {applied} — error: {error}")
            }
            (Some(BackdropState::Delegated(applied)), Some(error)) => {
                format!("delegated: {applied} — error: {error}")
            }
            (None, Some(error)) => format!("error: {error}"),
            (None, None) => "no backdrop applied yet".to_string(),
        };

        let note = widget::container(
            widget::column::with_capacity(12)
                .push(widget::text::title3("Glass spike"))
                .push(widget::text::body(
                    "Body text sits on a nearly opaque surface so it stays readable, \
                     while the sidebar and the header stay translucent. The backdrop \
                     behind them is provided by the compositor.",
                ))
                .push(widget::text::body(
                    "Sphinx of black quartz, judge my vow. The quick brown fox jumps \
                     over the lazy dog while the compositor smears the desktop \
                     wallpaper into a soft glow.",
                ))
                .push(buttons)
                .push(widget::text::caption(status)),
        )
        .padding(20)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(move |_theme| cosmic::widget::container::Style {
            background: Some(Background::Color(Color::from_rgba(
                0.08, 0.09, 0.11, note_alpha,
            ))),
            text_color: Some(text_color),
            ..cosmic::widget::container::Style::default()
        });

        widget::column::with_capacity(8)
            .push(
                widget::container(
                    widget::row::with_capacity(8)
                        .push(widget::text::title4("Acrylic Notes"))
                        .push(widget::Space::new().width(Length::Fill))
                        .push(widget::text::caption("M0 glass spike")),
                )
                .padding(12)
                .width(Length::Fill)
                .style(move |_theme| cosmic::widget::container::Style {
                    background: Some(Background::Color(Color::from_rgba(
                        0.09,
                        0.10,
                        0.12,
                        chrome_alpha,
                    ))),
                    text_color: Some(text_color),
                    ..cosmic::widget::container::Style::default()
                }),
            )
            .push(widget::row::with_capacity(0).push(sidebar).push(note))
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    fn header_start(&self) -> Vec<Element<'_, Self::Message>> {
        Vec::new()
    }
}
