// App framework - common types and trait for all apps/games

#[cfg(feature = "app-launcher")]
use embedded_graphics::pixelcolor::Rgb565;
#[cfg(feature = "app-launcher")]
use embedded_graphics::prelude::DrawTarget;

#[cfg(feature = "app-launcher")]
use crate::peripherals::touch::{SwipeDirection, TouchPoint};

#[cfg(feature = "flappy")]
pub mod flappy;
#[cfg(feature = "game-2048")]
pub mod game2048;
#[cfg(feature = "maze")]
pub mod maze;
#[cfg(feature = "mp3-player")]
pub mod mp3player;
#[cfg(feature = "settings")]
pub mod settings;
#[cfg(feature = "smart-home")]
pub mod smarthome;
#[cfg(feature = "snake")]
pub mod snake;
#[cfg(feature = "tetris")]
pub mod tetris;

/// Input state passed to apps each frame
#[cfg(feature = "app-launcher")]
pub struct AppInput {
    pub touch: Option<TouchPoint>,
    pub swipe: Option<SwipeDirection>,
    pub tap: bool,
    pub accel: (f32, f32, f32),
    pub dt_ms: u32, // milliseconds since last frame
}

/// Result of an app update
#[cfg(feature = "app-launcher")]
pub enum AppResult {
    Continue,
    Exit, // Return to launcher/watchface
}

/// Common trait for all apps/games
#[cfg(feature = "app-launcher")]
pub trait App {
    fn name(&self) -> &str;
    fn setup(&mut self);
    fn update(&mut self, input: &AppInput) -> AppResult;
    fn render<D: DrawTarget<Color = Rgb565>>(&self, d: &mut D);
}

/// All available app states
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum AppState {
    Watchface,
    #[cfg(feature = "app-launcher")]
    Launcher,
    #[cfg(feature = "snake")]
    Snake,
    #[cfg(feature = "game-2048")]
    Game2048,
    #[cfg(feature = "tetris")]
    Tetris,
    #[cfg(feature = "flappy")]
    Flappy,
    #[cfg(feature = "maze")]
    Maze,
    #[cfg(feature = "mp3-player")]
    Mp3Player,
    #[cfg(feature = "smart-home")]
    SmartHome,
    #[cfg(feature = "settings")]
    Settings,
}
