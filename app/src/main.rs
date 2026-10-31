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
    applied: Option<GlassStyle>,
    error: Option<String>,
}

#[derive(Clone, Debug)]
enum Message {
    Request(GlassStyle),
    Applied(GlassStyle),
    Failed(String),
}

impl App {
    /// Ask the platform for a backdrop and report back what it managed.
    fn request_backdrop(style: GlassStyle) -> cosmic::app::Task<Message> {
        // On Wayland, libcosmic uses its native backend which doesn't implement
        // `run_with_handle`. The compositor blur is applied automatically when
        // the window is transparent and the compositor supports the protocol.
        // We'll just report success for Blur style if the compositor reports
        // support, otherwise we'd need the winit backend.
        cosmic::iced::window::run_with_handle(cosmic::iced::window::Id::RESERVED, move |handle| {
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
        self.requested != GlassStyle::Opaque && self.applied != Some(GlassStyle::Opaque)
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
            applied: None,
            error: None,
        };
        app.core.set_header_title("Acrylic Notes".to_string());

        let task = Self::request_backdrop(app.requested);

        (app, task)
    }

    fn update(&mut self, message: Self::Message) -> cosmic::app::Task<Self::Message> {
        match message {
            Message::Request(style) => {
                self.requested = style;
                self.error = None;
                Self::request_backdrop(style)
            }
            Message::Applied(style) => {
                self.applied = Some(style);
                let id = cosmic::iced::window::Id::RESERVED;
                if style == GlassStyle::Blur {
                    cosmic::iced::window::enable_blur(id)
                } else {
                    cosmic::iced::window::disable_blur(id)
                }
            }
            Message::Failed(error) => {
                self.error = Some(error);
                cosmic::app::Task::none()
            }
        }
    }

    fn subscription(&self) -> cosmic::iced::Subscription<Self::Message> {
        #[cfg(target_os = "linux")]
        {
            // On Wayland, run_with_handle doesn't complete, so we use a one-shot
            // timer to reflect the compositor blur state if it was already applied.
            if self.requested == GlassStyle::Blur
                && self.applied.is_none()
                && self.capabilities.preferred == GlassStyle::Blur
            {
                return cosmic::iced::time::every(std::time::Duration::from_millis(500))
                    .map(|_| Message::Applied(GlassStyle::Blur));
            }
        }
        cosmic::iced::Subscription::none()
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

        let status = match (&self.applied, &self.error) {
            (Some(applied), None) => format!(
                "requested: {} — applied: {} — platform prefers: {}",
                self.requested.name(),
                applied.name(),
                self.capabilities.preferred.name(),
            ),
            (Some(applied), Some(error)) => {
                format!("applied: {applied} — error: {error}")
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
