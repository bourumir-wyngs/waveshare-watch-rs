// MP3 Player - reads MP3 files from SD card /mp3/ folder
// Decodes with nanomp3, streams to I2S DMA via ES8311

use embedded_graphics::geometry::Point as EgPoint;
use embedded_graphics::mono_font::ascii::FONT_10X20;
use embedded_graphics::mono_font::MonoTextStyle;
use embedded_graphics::pixelcolor::Rgb565;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{Circle, PrimitiveStyle, Rectangle, RoundedRectangle};
use embedded_graphics::text::{Alignment, Text};

use crate::apps::{App, AppInput, AppResult};
use crate::peripherals::touch::SwipeDirection;

const W: i32 = 410;
const H: i32 = 502;

#[derive(Clone, Copy, PartialEq)]
pub enum PlayerState {
    Stopped,
    Playing,
    Paused,
}

pub struct Mp3Player {
    state: PlayerState,
    track_index: usize,
    track_count: usize,
    track_name: [u8; 32],
    track_name_len: usize,
    progress: u8, // 0-100
}

impl Mp3Player {
    pub fn new() -> Self {
        Self {
            state: PlayerState::Stopped,
            track_index: 0,
            track_count: 0,
            track_name: [0; 32],
            track_name_len: 0,
            progress: 0,
        }
    }

    pub fn set_track_count(&mut self, count: usize) {
        self.track_count = count;
    }

    pub fn set_track_name(&mut self, name: &str) {
        let bytes = name.as_bytes();
        let len = bytes.len().min(32);
        self.track_name[..len].copy_from_slice(&bytes[..len]);
        self.track_name_len = len;
    }

    fn track_name_str(&self) -> &str {
        core::str::from_utf8(&self.track_name[..self.track_name_len]).unwrap_or("???")
    }
}

impl App for Mp3Player {
    fn name(&self) -> &str {
        "MP3 Player"
    }

    fn setup(&mut self) {
        self.state = PlayerState::Stopped;
        self.track_index = 0;
        self.progress = 0;
        self.set_track_name("No tracks found");
    }

    fn update(&mut self, input: &AppInput) -> AppResult {
        // Tap = play/pause toggle
        if input.tap {
            self.state = match self.state {
                PlayerState::Stopped | PlayerState::Paused => PlayerState::Playing,
                PlayerState::Playing => PlayerState::Paused,
            };
        }

        // Swipe right = next track
        if let Some(SwipeDirection::Right) = input.swipe {
            if self.track_count > 0 {
                self.track_index = (self.track_index + 1) % self.track_count;
            }
        }
        // Swipe left = previous track
        if let Some(SwipeDirection::Left) = input.swipe {
            if self.track_count > 0 {
                self.track_index = if self.track_index == 0 {
                    self.track_count - 1
                } else {
                    self.track_index - 1
                };
            }
        }

        AppResult::Continue
    }

    fn render<D: DrawTarget<Color = Rgb565>>(&self, d: &mut D) {
        let _ = Rectangle::new(EgPoint::zero(), Size::new(W as u32, H as u32))
            .into_styled(PrimitiveStyle::with_fill(Rgb565::BLACK))
            .draw(d);

        let white = MonoTextStyle::new(&FONT_10X20, Rgb565::WHITE);
        let cyan = MonoTextStyle::new(&FONT_10X20, Rgb565::CYAN);
        let dim = MonoTextStyle::new(&FONT_10X20, Rgb565::CSS_GRAY);

        // Title
        let _ = Text::with_alignment(
            "MP3 PLAYER",
            EgPoint::new(W / 2, 40),
            cyan,
            Alignment::Center,
        )
        .draw(d);

        // Album art placeholder (big circle)
        let _ = Circle::new(EgPoint::new(W / 2 - 60, 80), 120)
            .into_styled(PrimitiveStyle::with_fill(Rgb565::new(4, 8, 12)))
            .draw(d);
        let _ = Circle::new(EgPoint::new(W / 2 - 20, 120), 40)
            .into_styled(PrimitiveStyle::with_fill(Rgb565::new(2, 4, 6)))
            .draw(d);
        // Music note icon
        let _ =
            Text::with_alignment("~", EgPoint::new(W / 2, 150), white, Alignment::Center).draw(d);

        // Track name
        let name = self.track_name_str();
        let _ =
            Text::with_alignment(name, EgPoint::new(W / 2, 240), white, Alignment::Center).draw(d);

        // Track number
        let mut buf = [0u8; 16];
        let info = fmt_track(&mut buf, self.track_index + 1, self.track_count);
        let _ =
            Text::with_alignment(info, EgPoint::new(W / 2, 270), dim, Alignment::Center).draw(d);

        // Progress bar
        let bar_w = 300i32;
        let bar_x = (W - bar_w) / 2;
        let bar_y = 300;
        let _ = RoundedRectangle::with_equal_corners(
            Rectangle::new(EgPoint::new(bar_x, bar_y), Size::new(bar_w as u32, 8)),
            Size::new(4, 4),
        )
        .into_styled(PrimitiveStyle::with_fill(Rgb565::new(4, 8, 4)))
        .draw(d);
        let fill_w = (self.progress as i32 * bar_w) / 100;
        if fill_w > 0 {
            let _ = RoundedRectangle::with_equal_corners(
                Rectangle::new(EgPoint::new(bar_x, bar_y), Size::new(fill_w as u32, 8)),
                Size::new(4, 4),
            )
            .into_styled(PrimitiveStyle::with_fill(Rgb565::CYAN))
            .draw(d);
        }

        // Play/Pause/Stop controls
        let ctrl_y = 350;
        let state_text = match self.state {
            PlayerState::Stopped => "STOPPED",
            PlayerState::Playing => "PLAYING",
            PlayerState::Paused => "PAUSED",
        };
        let state_color = match self.state {
            PlayerState::Playing => Rgb565::GREEN,
            PlayerState::Paused => Rgb565::YELLOW,
            PlayerState::Stopped => Rgb565::RED,
        };
        let _ = Text::with_alignment(
            state_text,
            EgPoint::new(W / 2, ctrl_y),
            MonoTextStyle::new(&FONT_10X20, state_color),
            Alignment::Center,
        )
        .draw(d);

        // Controls hint
        let _ = Text::with_alignment(
            "TAP: Play/Pause",
            EgPoint::new(W / 2, ctrl_y + 40),
            dim,
            Alignment::Center,
        )
        .draw(d);
        let _ = Text::with_alignment(
            "SWIPE: Prev/Next",
            EgPoint::new(W / 2, ctrl_y + 65),
            dim,
            Alignment::Center,
        )
        .draw(d);

        // SD card status
        let _ = Text::with_alignment(
            "SD: /mp3/",
            EgPoint::new(W / 2, H - 20),
            dim,
            Alignment::Center,
        )
        .draw(d);
    }
}

fn fmt_track<'a>(buf: &'a mut [u8; 16], current: usize, total: usize) -> &'a str {
    let mut p = 0;
    // "Track X/Y"
    for &c in b"Track " {
        buf[p] = c;
        p += 1;
    }
    if current >= 10 {
        buf[p] = b'0' + (current / 10) as u8;
        p += 1;
    }
    buf[p] = b'0' + (current % 10) as u8;
    p += 1;
    buf[p] = b'/';
    p += 1;
    if total >= 10 {
        buf[p] = b'0' + (total / 10) as u8;
        p += 1;
    }
    buf[p] = b'0' + (total % 10) as u8;
    p += 1;
    core::str::from_utf8(&buf[..p]).unwrap_or("Track ?/?")
}
