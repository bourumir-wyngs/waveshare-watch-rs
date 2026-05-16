#![no_std]
#![no_main]

extern crate alloc;

mod apps;
#[cfg(feature = "audio")]
mod audio_clip;
mod board;
mod drivers;
mod peripherals;
mod ui;

use core::cell::RefCell;

use embedded_hal_bus::i2c::RefCellDevice;
use esp_alloc as _;
use esp_backtrace as _;

esp_bootloader_esp_idf::esp_app_desc!();

use embassy_executor::Spawner;
use embassy_time::{Duration, Instant, Timer};
use embedded_graphics::pixelcolor::Rgb565;
use embedded_graphics::prelude::RgbColor;

use esp_hal::delay::Delay;
use esp_hal::dma::{DmaRxBuf, DmaTxBuf};
use esp_hal::dma_buffers;
// use esp_hal::i2s::master::{I2s, Config as I2sConfig, DataFormat}; // TODO: wire I2S
use esp_hal::gpio::{Input, InputConfig, Level, Output, OutputConfig, Pull};
use esp_hal::i2c::master::{Config as I2cConfig, I2c};
use esp_hal::rtc_cntl::{
    sleep::{Ext0WakeupSource, TimerWakeupSource, WakeupLevel},
    wakeup_cause, Rtc as EspRtc,
};
use esp_hal::spi::master::{Config as SpiConfig, Spi};
use esp_hal::spi::Mode as SpiMode;
use esp_hal::system::SleepSource;
use esp_hal::time::Rate;
use esp_hal::timer::timg::TimerGroup;
use esp_println::println;

#[cfg(feature = "flappy")]
use crate::apps::flappy::FlappyGame;
#[cfg(feature = "game-2048")]
use crate::apps::game2048::Game2048;
#[cfg(feature = "maze")]
use crate::apps::maze::MazeGame;
#[cfg(feature = "mp3-player")]
use crate::apps::mp3player::Mp3Player;
#[cfg(feature = "settings")]
use crate::apps::settings::SettingsApp;
#[cfg(feature = "smart-home")]
use crate::apps::smarthome::SmartHomeApp;
#[cfg(feature = "snake")]
use crate::apps::snake::SnakeGame;
#[cfg(feature = "tetris")]
use crate::apps::tetris::TetrisGame;
#[cfg(feature = "snake")]
use crate::apps::AppResult;
use crate::apps::AppState;
#[cfg(feature = "app-launcher")]
use crate::apps::{App, AppInput};
use crate::drivers::co5300::Co5300Display;
use crate::drivers::framebuffer::Framebuffer;
use crate::drivers::qspi_bus::QspiBus;
#[cfg(feature = "audio")]
use crate::peripherals::audio::Es8311;
use crate::peripherals::imu::Qmi8658Imu;
use crate::peripherals::power::Axp2101Power;
use crate::peripherals::power_stats::{DisplayState, PowerStats, WifiMode};
use crate::peripherals::rtc::Pcf85063aRtc;
use crate::peripherals::touch::{Ft3168Touch, SwipeDirection};
#[cfg(feature = "app-launcher")]
use crate::ui::launcher::Launcher;
use crate::ui::pages::{self, Page};
use crate::ui::power_page;
use crate::ui::watchface::WatchFace;

const AOD_BRIGHTNESS: u8 = 0xCC; // ~80%
const AOD_DURATION_SECS: u64 = 7;
const AOD_ALERT_WINDOW_SECS: u64 = 2 * 60;
const SCREEN_OFF_HOUSEKEEPING_SECS: u64 = 600;
const STARTUP_BEEP_TEST: bool = false;
#[cfg(feature = "audio")]
const AUDIO_SAMPLE_RATE_HZ: u32 = audio_clip::SAMPLE_RATE_HZ;
#[cfg(feature = "audio")]
const AUDIO_DMA_BUFFER_BYTES: usize = 4000;
#[cfg(feature = "audio")]
const AUDIO_CLIP_I2S_BYTES: usize = audio_clip::SAMPLES.len() * 4;
#[cfg(feature = "audio")]
const AUDIO_CLIP_MS: u32 = audio_clip::DURATION_MS;
#[cfg(feature = "audio")]
const AUDIO_SAMPLE_GAIN: i32 = 256; //2048;
#[cfg(feature = "audio")]
const AUDIO_ALERT_FIRST_CLIPS: u8 = 0;
#[cfg(feature = "audio")]
const AUDIO_ALERT_MIDDLE_SILENCE_REPEATS: u8 = 6;
#[cfg(feature = "audio")]
const AUDIO_ALERT_LAST_CLIPS: u8 = 1;
#[cfg(feature = "audio")]
const AUDIO_ALERT_TAIL_SILENCE_REPEATS: u8 = 1;
#[cfg(feature = "audio")]
const AUDIO_LOUD_START_FREQ_HZ: u32 = 2_600;
#[cfg(feature = "audio")]
const AUDIO_LOUD_END_FREQ_HZ: u32 = 3_200;
#[cfg(feature = "audio")]
const AUDIO_LOUD_PATTERN_MS: u32 = 250;
#[cfg(feature = "audio")]
const AUDIO_LOUD_TONE_MS: u32 = 160;
#[cfg(feature = "audio")]
const AUDIO_LOUD_ATTACK_MS: u32 = 10;
#[cfg(feature = "audio")]
const AUDIO_LOUD_RELEASE_MS: u32 = 15;
#[cfg(feature = "audio")]
const AUDIO_LOUD_SIGNAL_MS: u32 = 2_000;
#[cfg(feature = "audio")]
const AUDIO_LOUD_PATTERN_FRAMES: usize =
    (AUDIO_SAMPLE_RATE_HZ as usize * AUDIO_LOUD_PATTERN_MS as usize) / 1_000;
#[cfg(feature = "audio")]
const AUDIO_LOUD_DMA_BUFFER_BYTES: usize = AUDIO_LOUD_PATTERN_FRAMES * 4;
#[cfg(feature = "audio")]
const AUDIO_LOUD_SIGNAL_I2S_REPEATS: u8 = (AUDIO_LOUD_SIGNAL_MS / AUDIO_LOUD_PATTERN_MS) as u8;
#[cfg(feature = "audio")]
const AUDIO_Q15_SCALE: i32 = 32_767;
#[cfg(feature = "audio")]
const AUDIO_LOUD_PEAK_Q15: i32 = 29_000;

// Network runner task (must be spawned for WiFi to work)
#[embassy_executor::task]
async fn net_task(
    mut runner: embassy_net::Runner<'static, esp_radio::wifi::WifiDevice<'static>>,
) -> ! {
    runner.run().await
}

// Simple NTP sync (UDP to pool.ntp.org:123)
async fn ntp_sync(
    stack: embassy_net::Stack<'static>,
    rtc: &mut crate::peripherals::rtc::Pcf85063aRtc<impl embedded_hal::i2c::I2c>,
) -> Result<(), &'static str> {
    use embassy_net::udp::{PacketMetadata, UdpSocket};

    let mut rx_meta = [PacketMetadata::EMPTY; 1];
    let mut rx_buf = [0u8; 256];
    let mut tx_meta = [PacketMetadata::EMPTY; 1];
    let mut tx_buf = [0u8; 256];

    let mut socket = UdpSocket::new(stack, &mut rx_meta, &mut rx_buf, &mut tx_meta, &mut tx_buf);
    socket.bind(12345).map_err(|_| "Bind failed")?;

    // NTP request packet (simplified: 48 bytes, first byte = 0x1B for client mode)
    let mut ntp_request = [0u8; 48];
    ntp_request[0] = 0x1B; // LI=0, VN=3, Mode=3 (client)

    // Resolve pool.ntp.org (use Google's NTP IP directly: 216.239.35.0)
    let ntp_addr = embassy_net::Ipv4Address::new(216, 239, 35, 0);
    socket
        .send_to(&ntp_request, (ntp_addr, 123))
        .await
        .map_err(|_| "Send failed")?;

    // Wait for response (timeout 5s)
    let mut response = [0u8; 48];
    match embassy_time::with_timeout(Duration::from_secs(5), socket.recv_from(&mut response)).await
    {
        Ok(Ok((len, _addr))) if len >= 48 => {
            // Parse NTP timestamp (bytes 40-43 = seconds since 1900-01-01)
            let ntp_secs =
                u32::from_be_bytes([response[40], response[41], response[42], response[43]]);
            // Convert NTP epoch (1900) to Unix epoch (1970): subtract 70 years in seconds
            let unix_secs = ntp_secs.wrapping_sub(2_208_988_800);
            // Convert to hours/minutes/seconds (UTC+2 for France)
            let utc_offset = 2 * 3600; // CEST (summer time)
            let local_secs = unix_secs + utc_offset;
            let time_of_day = local_secs % 86400;
            let hours = (time_of_day / 3600) as u8;
            let minutes = ((time_of_day % 3600) / 60) as u8;
            let seconds = (time_of_day % 60) as u8;

            // Calculate date (simplified: days since epoch)
            let total_days = (local_secs / 86400) as i32;
            // Simple date from days since 1970-01-01
            let (year, month, day) = days_to_date(total_days);

            println!(
                "[NTP] Time: {:02}:{:02}:{:02} {:02}/{:02}/{}",
                hours, minutes, seconds, day, month, year
            );

            // Set RTC
            let weekday =
                time_manager::weekday_from_date(year as i32, month as u8, day as u8).unwrap_or(0);
            let mut dt = crate::peripherals::rtc::DateTime::new(
                (year - 2000) as u8,
                month as u8,
                day as u8,
                hours,
                minutes,
                seconds,
            );
            dt.weekday = weekday;
            let _ = rtc.set_time(&dt);
            Ok(())
        }
        Ok(Ok(_)) => Err("Invalid response length"),
        Ok(Err(_)) => Err("Receive failed"),
        Err(_) => Err("Timeout"),
    }
}

fn days_to_date(days_since_epoch: i32) -> (u32, u32, u32) {
    // Simplified date calculation from days since 1970-01-01
    let mut y = 1970i32;
    let mut remaining = days_since_epoch;
    loop {
        let days_in_year = if y % 4 == 0 && (y % 100 != 0 || y % 400 == 0) {
            366
        } else {
            365
        };
        if remaining < days_in_year {
            break;
        }
        remaining -= days_in_year;
        y += 1;
    }
    let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    let month_days = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut m = 0;
    while m < 12 && remaining >= month_days[m] {
        remaining -= month_days[m];
        m += 1;
    }
    (y as u32, (m + 1) as u32, (remaining + 1) as u32)
}

fn rtc_to_time_manager_datetime(dt: crate::peripherals::rtc::DateTime) -> time_manager::DateTime {
    time_manager::DateTime::new(
        2000 + dt.year as i32,
        dt.month,
        dt.day,
        dt.hours,
        dt.minutes,
        dt.seconds,
    )
}

const RTC_ALERT_RGB: time_manager::Rgb = time_manager::Rgb::new(255, 72, 0);

fn schedule_records_with_rtc_alert(
    rtc_alert_time: Option<(u8, u8)>,
) -> ([time_manager::TimeRecord; 6], usize) {
    let empty = time_manager::TimeRecord::new(0, 0, [time_manager::WEEKDAY_NONE; 7]);
    let mut records = [empty; 6];

    for (idx, record) in time_manager::schedule::WAKE_SCHEDULE.iter().enumerate() {
        records[idx] = *record;
    }

    let mut len = time_manager::schedule::WAKE_SCHEDULE.len();
    if let Some((hours, minutes)) = rtc_alert_time {
        records[len] = time_manager::TimeRecord::new_with_color(
            hours.min(23),
            minutes.min(59),
            time_manager::schedule::EVERY_DAY,
            RTC_ALERT_RGB,
        );
        len += 1;
    }

    (records, len)
}

fn next_wake_with_rtc_alert(
    now: time_manager::DateTime,
    rtc_alert_time: Option<(u8, u8)>,
    minimum_duration: Option<core::time::Duration>,
) -> Result<Option<time_manager::NextWake>, time_manager::Error> {
    let (records, len) = schedule_records_with_rtc_alert(rtc_alert_time);
    let records = &records[..len];
    if let Some(minimum_duration) = minimum_duration {
        time_manager::next_wake_after(now, records, minimum_duration)
    } else {
        time_manager::next_wake(now, records)
    }
}

fn aod_next_wake_time(
    dt: crate::peripherals::rtc::DateTime,
    rtc_alert_time: Option<(u8, u8)>,
) -> Option<(u8, u8)> {
    const MAX_AOD_WAKE_SECS: u64 = 24 * 60 * 60;

    let now = rtc_to_time_manager_datetime(dt);
    match next_wake_with_rtc_alert(now, rtc_alert_time, None) {
        Ok(Some(wake)) if wake.duration.as_secs() <= MAX_AOD_WAKE_SECS => {
            Some((wake.hour, wake.minute))
        }
        _ => None,
    }
}

fn aod_scheduled_time_alert_color(
    dt: crate::peripherals::rtc::DateTime,
    rtc_alert_time: Option<(u8, u8)>,
) -> Option<Rgb565> {
    let now = rtc_to_time_manager_datetime(dt);
    let (records, len) = schedule_records_with_rtc_alert(rtc_alert_time);
    match time_manager::closest_wake(now, &records[..len]) {
        Ok(Some(wake)) if wake.difference.as_secs() < AOD_ALERT_WINDOW_SECS => {
            Some(schedule_rgb_to_rgb565(wake.color))
        }
        _ => None,
    }
}

fn rtc_alert_due(dt: crate::peripherals::rtc::DateTime, rtc_alert_time: Option<(u8, u8)>) -> bool {
    let Some((hours, minutes)) = rtc_alert_time else {
        return false;
    };
    let record = time_manager::TimeRecord::new_with_color(
        hours.min(23),
        minutes.min(59),
        time_manager::schedule::EVERY_DAY,
        RTC_ALERT_RGB,
    );
    match time_manager::closest_wake(rtc_to_time_manager_datetime(dt), &[record]) {
        Ok(Some(wake)) => wake.difference.as_secs() < AOD_ALERT_WINDOW_SECS,
        _ => false,
    }
}

fn clear_quick_view_alert_entry(
    quick_view_time_digits: &mut [Option<u8>; 4],
    quick_view_time_len: &mut usize,
    quick_view_show_set_key: &mut bool,
) {
    *quick_view_time_digits = [None; 4];
    *quick_view_time_len = 0;
    *quick_view_show_set_key = true;
}

fn sync_quick_view_alert_entry(
    rtc_alert_time: Option<(u8, u8)>,
    quick_view_time_digits: &mut [Option<u8>; 4],
    quick_view_time_len: &mut usize,
    quick_view_show_set_key: &mut bool,
) {
    if let Some((hours, minutes)) = rtc_alert_time {
        *quick_view_time_digits = [
            Some(hours / 10),
            Some(hours % 10),
            Some(minutes / 10),
            Some(minutes % 10),
        ];
        *quick_view_time_len = quick_view_time_digits.len();
        *quick_view_show_set_key = false;
    } else {
        clear_quick_view_alert_entry(
            quick_view_time_digits,
            quick_view_time_len,
            quick_view_show_set_key,
        );
    }
}

fn schedule_rgb_to_rgb565(color: time_manager::Rgb) -> Rgb565 {
    Rgb565::new(color.red >> 3, color.green >> 2, color.blue >> 3)
}

#[cfg(feature = "audio")]
fn load_audio_clip_i2s(buf: &mut [u8; AUDIO_DMA_BUFFER_BYTES]) {
    for (frame, mono) in audio_clip::SAMPLES.iter().copied().enumerate() {
        let dst = frame * 4;
        if dst + 3 >= buf.len() {
            break;
        }
        let sample =
            ((mono as i32) * AUDIO_SAMPLE_GAIN).clamp(i16::MIN as i32, i16::MAX as i32) as i16;
        let bytes = sample.to_le_bytes();
        buf[dst] = bytes[0];
        buf[dst + 1] = bytes[1];
        buf[dst + 2] = bytes[0];
        buf[dst + 3] = bytes[1];
    }
}

#[cfg(feature = "audio")]
const AUDIO_SINE_TABLE_Q15: [i16; 32] = [
    0, 6393, 12540, 18204, 23170, 27246, 30273, 32138, 32767, 32138, 30273, 27246, 23170, 18204,
    12540, 6393, 0, -6393, -12540, -18204, -23170, -27246, -30273, -32138, -32767, -32138, -30273,
    -27246, -23170, -18204, -12540, -6393,
];

#[cfg(feature = "audio")]
fn sine_q15(phase: u32) -> i32 {
    let idx = (phase >> 27) as usize;
    let next_idx = (idx + 1) & (AUDIO_SINE_TABLE_Q15.len() - 1);
    let frac = ((phase >> 11) & 0xffff) as i64;
    let a = AUDIO_SINE_TABLE_Q15[idx] as i64;
    let b = AUDIO_SINE_TABLE_Q15[next_idx] as i64;

    (a + (((b - a) * frac) >> 16)) as i32
}

#[cfg(feature = "audio")]
fn load_loud_alert_i2s(buf: &mut [u8; AUDIO_LOUD_DMA_BUFFER_BYTES]) {
    let tone_frames = (AUDIO_SAMPLE_RATE_HZ * AUDIO_LOUD_TONE_MS / 1_000) as usize;
    let attack_frames = (AUDIO_SAMPLE_RATE_HZ * AUDIO_LOUD_ATTACK_MS / 1_000) as usize;
    let release_frames = (AUDIO_SAMPLE_RATE_HZ * AUDIO_LOUD_RELEASE_MS / 1_000) as usize;
    let chirp_denominator = tone_frames.saturating_sub(1).max(1) as u64;
    let chirp_span_hz = AUDIO_LOUD_END_FREQ_HZ - AUDIO_LOUD_START_FREQ_HZ;
    let mut phase = 0u32;

    for frame in 0..(buf.len() / 4) {
        let sample = if frame < tone_frames {
            let freq_hz = AUDIO_LOUD_START_FREQ_HZ
                + ((chirp_span_hz as u64 * frame as u64) / chirp_denominator) as u32;
            let phase_step = (((freq_hz as u64) << 32) / AUDIO_SAMPLE_RATE_HZ as u64) as u32;
            let envelope_q15 = if attack_frames > 0 && frame < attack_frames {
                (frame as i64 * AUDIO_Q15_SCALE as i64) / attack_frames as i64
            } else if release_frames > 0 && frame + release_frames >= tone_frames {
                let remaining_frames = tone_frames - frame;
                ((remaining_frames as i64 * AUDIO_Q15_SCALE as i64) / release_frames as i64)
                    .min(AUDIO_Q15_SCALE as i64)
            } else {
                AUDIO_Q15_SCALE as i64
            };
            let sine = sine_q15(phase) as i64;
            phase = phase.wrapping_add(phase_step);

            ((sine * AUDIO_LOUD_PEAK_Q15 as i64 * envelope_q15)
                / (AUDIO_Q15_SCALE as i64 * AUDIO_Q15_SCALE as i64))
                .clamp(i16::MIN as i64, i16::MAX as i64) as i16
        } else {
            0
        };
        let bytes = sample.to_le_bytes();
        let dst = frame * 4;
        buf[dst] = bytes[0];
        buf[dst + 1] = bytes[1];
        buf[dst + 2] = bytes[0];
        buf[dst + 3] = bytes[1];
    }
}

fn app_needs_motion_imu(app_state: AppState) -> bool {
    match app_state {
        #[cfg(feature = "maze")]
        AppState::Maze => true,
        #[cfg(feature = "tetris")]
        AppState::Tetris => true,
        #[cfg(feature = "flappy")]
        AppState::Flappy => true,
        _ => false,
    }
}

/// Take a cheap snapshot of the current power state into the shared
/// `PowerStats` struct. Called from the Power page renderer (and from the
/// swipe preview) so the user sees a live read-out without us adding any
/// extra sampling. All inputs are already tracked by the main loop.
fn update_power_stats(
    stats: &mut PowerStats,
    screen_state: u8,
    imu_on: bool,
    wifi_connected: bool,
    wifi_on_request: bool,
    brightness: u8,
    batt_mv: u16,
    batt_pct: u8,
    charging: bool,
) {
    stats.display = Some(match screen_state {
        0 => DisplayState::Off,
        1 => DisplayState::Aod,
        2 => DisplayState::Dim,
        _ => DisplayState::Bright,
    });
    stats.wifi = Some(if !wifi_on_request && !wifi_connected {
        WifiMode::Off
    } else if wifi_connected {
        // set_power_save(Maximum) is applied via Config, so once connected
        // we're in WIFI_PS_MAX_MODEM.
        WifiMode::PowerSave
    } else {
        // Radio up but handshake still in progress.
        WifiMode::Active
    });
    stats.imu_on = imu_on;
    stats.brightness = brightness;
    // Audio/SD track "currently drawing current". The codec is in shutdown
    // except during a beep; the SD is not currently gated (see TODO in
    // main init). For now, report both off — flip these flags when the
    // main loop wakes the respective subsystem.
    stats.audio_on = false;
    stats.sd_on = false;
    stats.battery_mv = batt_mv;
    stats.battery_pct = batt_pct;
    stats.charging = charging;
}

#[esp_rtos::main]
async fn main(_spawner: Spawner) {
    // Heap: 200KB SRAM + PSRAM for large allocs
    // The BLE+WiFi coex radio stack needs ~100KB+ of internal SRAM for
    // btdm_controller_init tasks and buffers. 64KB was too small and
    // caused a StoreProhibited panic (null-pointer from failed alloc).
    esp_alloc::heap_allocator!(size: 200 * 1024);
    // Power-aware: default to 160MHz instead of 240MHz.
    // Saves ~30% CPU power without noticeable impact on UI/sensor work.
    // Game code can still trigger short bursts via DMA/peripherals at 80MHz QSPI which is unchanged.
    let mut peripherals =
        esp_hal::init(esp_hal::Config::default().with_cpu_clock(esp_hal::clock::CpuClock::_160MHz));

    // PSRAM
    esp_alloc::psram_allocator!(peripherals.PSRAM, esp_hal::psram);

    // Embassy timer
    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0);

    esp_println::logger::init_logger_from_env();
    println!("=== Waveshare Watch RS v0.4 (Embassy) ===");

    let delay = Delay::new();
    let mut esp_rtc = EspRtc::new(peripherals.LPWR);
    let boot_wakeup_cause = wakeup_cause();
    let woke_from_timer = matches!(boot_wakeup_cause, SleepSource::Timer);
    println!("[POWER] Wake cause: {:?}", boot_wakeup_cause);

    // === RESCUE DEAD I2C BUS ===
    // If the previous firmware turned off ALDO1, the I2C pullups are dead.
    // The hardware I2C controller will fail because the bus is stuck low.
    // We bit-bang a push-pull write to AXP2101 to turn ALDO1 back on.
    {
        // START
        {
            let mut sda_out = Output::new(
                peripherals.GPIO15.reborrow(),
                Level::High,
                OutputConfig::default(),
            );
            let mut scl_out = Output::new(
                peripherals.GPIO14.reborrow(),
                Level::High,
                OutputConfig::default(),
            );
            delay.delay_micros(50);
            sda_out.set_low();
            delay.delay_micros(10);
            scl_out.set_low();
            delay.delay_micros(10);
        }

        macro_rules! send_byte {
            ($byte:expr) => {
                for i in (0..8).rev() {
                    let _sda_out = Output::new(
                        peripherals.GPIO15.reborrow(),
                        if ($byte & (1 << i)) != 0 {
                            Level::High
                        } else {
                            Level::Low
                        },
                        OutputConfig::default(),
                    );
                    let mut scl_out = Output::new(
                        peripherals.GPIO14.reborrow(),
                        Level::Low,
                        OutputConfig::default(),
                    );
                    delay.delay_micros(10);
                    scl_out.set_high();
                    delay.delay_micros(10);
                    scl_out.set_low();
                    delay.delay_micros(10);
                }
                // ACK
                {
                    let _sda_in = Input::new(peripherals.GPIO15.reborrow(), InputConfig::default());
                    let mut scl_out = Output::new(
                        peripherals.GPIO14.reborrow(),
                        Level::Low,
                        OutputConfig::default(),
                    );
                    delay.delay_micros(10);
                    scl_out.set_high();
                    delay.delay_micros(10);
                    scl_out.set_low();
                    delay.delay_micros(10);
                }
                {
                    let _sda_out = Output::new(
                        peripherals.GPIO15.reborrow(),
                        Level::Low,
                        OutputConfig::default(),
                    );
                }
            };
        }

        send_byte!(0x68); // 0x34 << 1 | 0 (Write)
        send_byte!(0x90); // REG_LDO_ONOFF0
        send_byte!(0x01); // Enable ALDO1

        // STOP
        {
            let mut sda_out = Output::new(
                peripherals.GPIO15.reborrow(),
                Level::Low,
                OutputConfig::default(),
            );
            let mut scl_out = Output::new(
                peripherals.GPIO14.reborrow(),
                Level::Low,
                OutputConfig::default(),
            );
            delay.delay_micros(10);
            scl_out.set_high();
            delay.delay_micros(10);
            sda_out.set_high();
            delay.delay_micros(50);
        }
    }

    // === I2C Bus ===
    let i2c = I2c::new(
        peripherals.I2C0,
        I2cConfig::default().with_frequency(Rate::from_khz(400)),
    )
    .expect("I2C failed")
    .with_sda(peripherals.GPIO15)
    .with_scl(peripherals.GPIO14);
    let i2c_ref = RefCell::new(i2c);

    // === Power ===
    let mut power = Axp2101Power::new(RefCellDevice::new(&i2c_ref));
    let _ = power.init();
    // Drop the die-temp ADC channel we never show on the UI — shaves a
    // few hundred µA off the AXP2101 housekeeping load.
    let _ = power.trim_adc_channels();
    println!("[POWER] OK");

    // === Display 80MHz DMA ===
    let spi_config = SpiConfig::default()
        .with_frequency(Rate::from_mhz(80))
        .with_mode(SpiMode::_0);
    let (rx_buf, rx_desc, tx_buf, tx_desc) = dma_buffers!(8000);
    let dma_rx = DmaRxBuf::new(rx_desc, rx_buf).unwrap();
    let dma_tx = DmaTxBuf::new(tx_desc, tx_buf).unwrap();
    let spi = Spi::new(peripherals.SPI2, spi_config)
        .expect("SPI failed")
        .with_sck(peripherals.GPIO11)
        .with_sio0(peripherals.GPIO4)
        .with_sio1(peripherals.GPIO5)
        .with_sio2(peripherals.GPIO6)
        .with_sio3(peripherals.GPIO7)
        .with_dma(peripherals.DMA_CH0)
        .with_buffers(dma_rx, dma_tx);
    let cs = Output::new(peripherals.GPIO12, Level::High, OutputConfig::default());
    let reset = Output::new(peripherals.GPIO8, Level::High, OutputConfig::default());
    let mut display = Co5300Display::new(QspiBus::new(spi, cs), reset);
    display.init();

    // Enable Tearing Effect output on CO5300 (TE pin = GPIO13)
    // Command 0x35 = TEARON, param 0x00 = VBlank only
    display.bus_mut().write_c8d8(0x35, 0x00);
    let te_pin = Input::new(peripherals.GPIO13, InputConfig::default());
    println!("[DISPLAY] OK (TE VSync enabled)");

    // === Framebuffer PSRAM ===
    let mut fb = Framebuffer::new();
    fb.clear_color(Rgb565::BLACK);
    fb.flush(&mut display);
    println!("[FB] OK");

    // === RTC ===
    let mut rtc = Pcf85063aRtc::new(RefCellDevice::new(&i2c_ref));
    let _ = rtc.init();
    println!("[RTC] OK");

    // === State ===
    let mut watchface = WatchFace::new();
    watchface.wifi_connected = false; // radio stays off until user taps the button
    let mut current_page = Page::Aod;
    let mut quick_view_time_digits: [Option<u8>; 4] = [None; 4];
    let mut quick_view_time_len: usize = 0;
    let mut quick_view_show_set_key = true;
    let mut quick_view_loaded = false;
    let mut alarm_loud_enabled = rtc.loud_flag().unwrap_or(false);
    #[cfg(feature = "audio")]
    let alarm_loud_enabled_at_boot = alarm_loud_enabled;
    let mut rtc_alert_time = rtc.get_alert_time().unwrap_or(None);
    // Live power-diagnostic snapshot, updated in the main loop and read
    // by the Power page renderer. Kept as plain POD so reading it is free.
    let mut power_stats = PowerStats::new();
    power_stats.cpu_mhz = 160;
    #[cfg(feature = "app-launcher")]
    let mut app_state = AppState::Watchface;
    #[cfg(not(feature = "app-launcher"))]
    let mut app_state = AppState::Watchface;
    #[cfg(feature = "snake")]
    let mut snake_game = SnakeGame::new();
    #[cfg(feature = "game-2048")]
    let mut game_2048 = Game2048::new();
    #[cfg(feature = "tetris")]
    let mut tetris_game = TetrisGame::new();
    #[cfg(feature = "flappy")]
    let mut flappy_game = FlappyGame::new();
    #[cfg(feature = "maze")]
    let mut maze_game = MazeGame::new();
    #[cfg(feature = "app-launcher")]
    let mut launcher = Launcher::new();
    #[cfg(feature = "settings")]
    let mut settings_app = SettingsApp::new();
    #[cfg(feature = "mp3-player")]
    let mut mp3_player = Mp3Player::new();
    #[cfg(feature = "smart-home")]
    let mut smarthome_app = SmartHomeApp::new();
    let mut last_touch_y: u16 = 0;
    let mut last_touch_x: u16 = 0;
    let mut accel = (0.0f32, 0.0f32, 0.0f32);
    let mut gyro_data = (0i16, 0i16, 0i16);
    let mut imu_temp: i16 = 250;
    let mut batt_pct: u8 = 0;
    let mut batt_mv: u16 = 0;
    let mut charging = false;
    let mut page_dirty = true;
    let mut last_interaction = Instant::now();
    // screen_state levels:
    //   3 = full bright (interactive)
    //   1 = AOD (Always-On Display: super-dim, minimal HH:MM, 1 update / minute)
    //   0 = full off (DISPOFF + SLPIN)
    let mut screen_state: u8 = 1;
    let mut aod_entered_at: Instant;
    // Tracks the last minute we rendered in AOD so we update the screen exactly
    // once per minute, not faster. Saves both DMA bandwidth and AMOLED current.
    let mut aod_last_minute: u8 = 99;
    let mut boot_aod_time_alert = false;

    // First visible frame: show AOD before slower optional subsystem init.
    if let Ok(pct) = power.get_battery_percent() {
        batt_pct = pct;
        batt_mv = power.get_battery_voltage().unwrap_or(0);
        charging = power.is_charging().unwrap_or(false);
        watchface.update_battery(batt_pct, batt_mv, charging);
    }
    if let Ok(dt) = rtc.get_time() {
        watchface.update_time(dt.hours, dt.minutes, dt.seconds);
        watchface.update_date(dt.day, dt.month, dt.year, dt.weekday);
        watchface.update_next_wake_time(aod_next_wake_time(dt, rtc_alert_time));
        let alert_color = aod_scheduled_time_alert_color(dt, rtc_alert_time);
        boot_aod_time_alert = alert_color.is_some();
        watchface.update_aod_time_alert_color(alert_color);
        if rtc_alert_due(dt, rtc_alert_time) {
            let _ = rtc.disable_alert_time();
            rtc_alert_time = None;
            quick_view_loaded = false;
            clear_quick_view_alert_entry(
                &mut quick_view_time_digits,
                &mut quick_view_time_len,
                &mut quick_view_show_set_key,
            );
        }
        aod_last_minute = dt.minutes;
    }
    display.set_brightness(AOD_BRIGHTNESS);
    let _ = watchface.render_aod(&mut fb);
    fb.flush(&mut display);
    println!("[AOD] First frame displayed");

    // === Touch ===
    let mut touch_rst = Output::new(peripherals.GPIO9, Level::High, OutputConfig::default());
    // GPIO38 is the FT3168 INT line: held high by pull-up, pulled low by the controller
    // when a finger is on the screen. We use it both for level checks and as an async wake source.
    let mut touch_int = Input::new(
        peripherals.GPIO38,
        InputConfig::default().with_pull(Pull::Up),
    );
    touch_rst.set_low();
    delay.delay_millis(10);
    touch_rst.set_high();
    delay.delay_millis(50);
    let mut touch = Ft3168Touch::new(RefCellDevice::new(&i2c_ref));
    let _ = touch.init();
    println!("[TOUCH] OK");

    // === IMU ===
    let mut imu = Qmi8658Imu::new(RefCellDevice::new(&i2c_ref));
    let mut imu_initialized = false;
    println!("[IMU] Deferred");

    // === Lazy SD Card (SPI3) ===
    #[cfg(feature = "mp3-player")]
    let mut sd_tokens = Some((
        peripherals.SPI3,
        peripherals.GPIO2,
        peripherals.GPIO1,
        peripherals.GPIO3,
        peripherals.GPIO17,
    ));
    #[cfg(feature = "mp3-player")]
    let mut sd_scanned = false;
    #[cfg(feature = "mp3-player")]
    let mut mp3_files: alloc::vec::Vec<alloc::string::String> = alloc::vec::Vec::new();

    #[cfg(feature = "mp3-player")]
    macro_rules! ensure_mp3_scan {
        () => {{
            if !sd_scanned {
                sd_scanned = true;
                println!("[SD] Lazy init...");
                if let Some((spi3, gpio2, gpio1, gpio3, gpio17)) = sd_tokens.take() {
                    let sd_spi_config = SpiConfig::default()
                        .with_frequency(Rate::from_mhz(4))
                        .with_mode(SpiMode::_0);
                    let sd_spi = Spi::new(spi3, sd_spi_config)
                        .expect("SPI3 failed")
                        .with_sck(gpio2)
                        .with_mosi(gpio1)
                        .with_miso(gpio3);
                    let sd_cs = Output::new(gpio17, Level::High, OutputConfig::default());
                    let sd_spi_dev =
                        embedded_hal_bus::spi::ExclusiveDevice::new_no_delay(sd_spi, sd_cs)
                            .unwrap();
                    let sd_card = embedded_sdmmc::SdCard::new(sd_spi_dev, Delay::new());

                    match sd_card.num_bytes() {
                        Ok(size) => {
                            println!("[SD] Card {}MB", size / 1024 / 1024);

                            struct DummyTime;
                            impl embedded_sdmmc::TimeSource for DummyTime {
                                fn get_timestamp(&self) -> embedded_sdmmc::Timestamp {
                                    embedded_sdmmc::Timestamp::from_calendar(2026, 4, 6, 12, 0, 0)
                                        .unwrap()
                                }
                            }

                            let mut volume_mgr =
                                embedded_sdmmc::VolumeManager::new(sd_card, DummyTime);
                            match volume_mgr.open_raw_volume(embedded_sdmmc::VolumeIdx(0)) {
                                Ok(volume) => {
                                    if let Ok(root_dir) = volume_mgr.open_root_dir(volume) {
                                        if let Ok(mp3_dir) = volume_mgr.open_dir(root_dir, "MP3") {
                                            println!("[SD] Found /MP3/ folder");
                                            let _ = volume_mgr.iterate_dir(mp3_dir, |entry| {
                                                if !entry.attributes.is_directory() {
                                                    let name = core::str::from_utf8(
                                                        &entry.name.base_name(),
                                                    )
                                                    .unwrap_or("?");
                                                    let ext = core::str::from_utf8(
                                                        &entry.name.extension(),
                                                    )
                                                    .unwrap_or("");
                                                    let full = alloc::format!(
                                                        "{}.{}",
                                                        name.trim(),
                                                        ext.trim()
                                                    );
                                                    println!("[SD]   {}", full);
                                                    mp3_files.push(full);
                                                }
                                            });
                                            let _ = volume_mgr.close_dir(mp3_dir);
                                        } else if let Ok(mp3_dir) =
                                            volume_mgr.open_dir(root_dir, "mp3")
                                        {
                                            println!("[SD] Found /mp3/ folder");
                                            let _ = volume_mgr.iterate_dir(mp3_dir, |entry| {
                                                if !entry.attributes.is_directory() {
                                                    let name = core::str::from_utf8(
                                                        &entry.name.base_name(),
                                                    )
                                                    .unwrap_or("?");
                                                    let ext = core::str::from_utf8(
                                                        &entry.name.extension(),
                                                    )
                                                    .unwrap_or("");
                                                    let full = alloc::format!(
                                                        "{}.{}",
                                                        name.trim(),
                                                        ext.trim()
                                                    );
                                                    println!("[SD]   {}", full);
                                                    mp3_files.push(full);
                                                }
                                            });
                                            let _ = volume_mgr.close_dir(mp3_dir);
                                        } else {
                                            println!("[SD] No /mp3/ or /MP3/ folder");
                                        }
                                        let _ = volume_mgr.close_dir(root_dir);
                                    } else {
                                        println!("[SD] Can't open root dir");
                                    }
                                }
                                Err(e) => {
                                    println!("[SD] Can't open volume: {:?}", e);
                                }
                            }
                            println!("[SD] {} files found", mp3_files.len());
                        }
                        Err(_) => println!("[SD] No card"),
                    }
                } else {
                    println!("[SD] Scan unavailable");
                }
            }

            mp3_player.set_track_count(mp3_files.len());
            if !mp3_files.is_empty() {
                mp3_player.set_track_name(&mp3_files[0]);
            }
        }};
    }

    // === Lazy audio (ES8311 codec + I2S) ===
    #[cfg(feature = "audio")]
    let mut pa_en = Output::new(peripherals.GPIO46, Level::Low, OutputConfig::default());
    #[cfg(feature = "audio")]
    let mut audio_tokens = Some((
        peripherals.I2S0,
        peripherals.DMA_CH1,
        peripherals.GPIO16,
        peripherals.GPIO41,
        peripherals.GPIO45,
        peripherals.GPIO40,
    ));
    #[cfg(feature = "audio")]
    let mut audio_codec: Option<Es8311<RefCellDevice<'_, I2c<'static, esp_hal::Blocking>>>> = None;
    #[cfg(feature = "audio")]
    let mut i2s_tx: Option<esp_hal::i2s::master::I2sTx<'static, esp_hal::Blocking>> = None;
    #[cfg(feature = "audio")]
    let mut clip_buf: Option<&'static [u8; AUDIO_DMA_BUFFER_BYTES]> = None;
    #[cfg(feature = "audio")]
    let mut silence_buf: Option<&'static [u8; AUDIO_DMA_BUFFER_BYTES]> = None;
    #[cfg(feature = "audio")]
    let mut loud_buf: Option<&'static [u8; AUDIO_LOUD_DMA_BUFFER_BYTES]> = None;
    #[cfg(feature = "audio")]
    static I2S_TX_DESC: static_cell::StaticCell<[esp_hal::dma::DmaDescriptor; 8]> =
        static_cell::StaticCell::new();
    #[cfg(feature = "audio")]
    static CLIP_BUF: static_cell::StaticCell<[u8; AUDIO_DMA_BUFFER_BYTES]> =
        static_cell::StaticCell::new();
    #[cfg(feature = "audio")]
    static SILENCE_BUF: static_cell::StaticCell<[u8; AUDIO_DMA_BUFFER_BYTES]> =
        static_cell::StaticCell::new();
    #[cfg(feature = "audio")]
    static LOUD_BUF: static_cell::StaticCell<[u8; AUDIO_LOUD_DMA_BUFFER_BYTES]> =
        static_cell::StaticCell::new();

    #[cfg(feature = "audio")]
    macro_rules! ensure_audio {
        () => {{
            if audio_codec.is_none() {
                if let Some((i2s0, dma_ch1, gpio16, gpio41, gpio45, gpio40)) = audio_tokens.take() {
                    println!("[AUDIO] Lazy init codec + I2S...");
                    let mut codec = Es8311::new(RefCellDevice::new(&i2c_ref));
                    match codec.init() {
                        Ok(()) => println!("[AUDIO] Codec OK"),
                        Err(_) => println!("[AUDIO] Codec FAILED"),
                    }
                    let _ = codec.shutdown();

                    let i2s_config = esp_hal::i2s::master::Config::default()
                        .with_sample_rate(Rate::from_hz(AUDIO_SAMPLE_RATE_HZ))
                        .with_data_format(esp_hal::i2s::master::DataFormat::Data16Channel16);
                    let i2s_periph = esp_hal::i2s::master::I2s::new(i2s0, dma_ch1, i2s_config)
                        .expect("I2S failed")
                        .with_mclk(gpio16);
                    let tx = i2s_periph
                        .i2s_tx
                        .with_bclk(gpio41)
                        .with_ws(gpio45)
                        .with_dout(gpio40)
                        .build(I2S_TX_DESC.init([esp_hal::dma::DmaDescriptor::EMPTY; 8]));

                    let clip = CLIP_BUF.init_with(|| [0u8; AUDIO_DMA_BUFFER_BYTES]);
                    load_audio_clip_i2s(clip);
                    let silence = SILENCE_BUF.init_with(|| [0u8; AUDIO_DMA_BUFFER_BYTES]);
                    let loud = LOUD_BUF.init_with(|| [0u8; AUDIO_LOUD_DMA_BUFFER_BYTES]);
                    load_loud_alert_i2s(loud);
                    println!(
                        "[AUDIO] I2S OK (clip: {} mono-i8 samples, {} ms, {} active bytes -> {} DMA bytes)",
                        audio_clip::SAMPLES.len(),
                        AUDIO_CLIP_MS,
                        AUDIO_CLIP_I2S_BYTES.min(clip.len()),
                        clip.len()
                    );
                    audio_codec = Some(codec);
                    i2s_tx = Some(tx);
                    clip_buf = Some(clip);
                    silence_buf = Some(silence);
                    loud_buf = Some(loud);
                } else {
                    println!("[AUDIO] Init unavailable");
                }
            }

            let _ = clip_buf.is_some();
            let _ = loud_buf.is_some();
            audio_codec.is_some() && i2s_tx.is_some()
        }};
    }

    #[cfg(feature = "audio")]
    macro_rules! play_audio_sequence {
        (
            $label:expr,
            $first_clip_repeats:expr,
            $middle_silence_repeats:expr,
            $last_clip_repeats:expr,
            $tail_silence_repeats:expr
        ) => {{
            if ensure_audio!() {
                if let (Some(codec), Some(tx), Some(clip), Some(silence)) =
                    (audio_codec.as_mut(), i2s_tx.as_mut(), clip_buf, silence_buf)
                {
                    println!("[AUDIO] {}", $label);
                    let _ = codec.unmute();
                    pa_en.set_high();
                    delay.delay_millis(20);

                    for _ in 0..$first_clip_repeats {
                        match tx.write_dma(clip) {
                            Ok(transfer) => {
                                let _ = transfer.wait();
                            }
                            Err(_) => {
                                println!("[AUDIO] DMA start failed");
                                break;
                            }
                        }
                    }

                    for _ in 0..$middle_silence_repeats {
                        match tx.write_dma(silence) {
                            Ok(transfer) => {
                                let _ = transfer.wait();
                            }
                            Err(_) => {
                                println!("[AUDIO] DMA start failed");
                                break;
                            }
                        }
                    }

                    for _ in 0..$last_clip_repeats {
                        match tx.write_dma(clip) {
                            Ok(transfer) => {
                                let _ = transfer.wait();
                            }
                            Err(_) => {
                                println!("[AUDIO] DMA start failed");
                                break;
                            }
                        }
                    }

                    for _ in 0..$tail_silence_repeats {
                        match tx.write_dma(silence) {
                            Ok(transfer) => {
                                let _ = transfer.wait();
                            }
                            Err(_) => {
                                println!("[AUDIO] DMA start failed");
                                break;
                            }
                        }
                    }

                    delay.delay_millis(5);
                    pa_en.set_low();
                    let _ = codec.mute();
                }
            } else {
                println!("[AUDIO] Beep skipped; audio unavailable");
            }
        }};
    }

    #[cfg(feature = "audio")]
    macro_rules! play_loud_alert_signal {
        ($label:expr) => {{
            if ensure_audio!() {
                if let (Some(codec), Some(tx), Some(loud), Some(silence)) =
                    (audio_codec.as_mut(), i2s_tx.as_mut(), loud_buf, silence_buf)
                {
                    println!("[AUDIO] {}", $label);
                    let _ = codec.unmute();
                    pa_en.set_high();
                    delay.delay_millis(20);

                    for _ in 0..AUDIO_LOUD_SIGNAL_I2S_REPEATS {
                        match tx.write_dma(loud) {
                            Ok(transfer) => {
                                let _ = transfer.wait();
                            }
                            Err(_) => {
                                println!("[AUDIO] DMA start failed");
                                break;
                            }
                        }
                    }

                    match tx.write_dma(silence) {
                        Ok(transfer) => {
                            let _ = transfer.wait();
                        }
                        Err(_) => println!("[AUDIO] DMA start failed"),
                    }

                    delay.delay_millis(5);
                    pa_en.set_low();
                    let _ = codec.mute();
                }
            } else {
                println!("[AUDIO] Loud alert skipped; audio unavailable");
            }
        }};
    }

    #[cfg(feature = "audio")]
    if STARTUP_BEEP_TEST {
        play_audio_sequence!(
            "startup test clip",
            AUDIO_ALERT_FIRST_CLIPS,
            AUDIO_ALERT_MIDDLE_SILENCE_REPEATS,
            AUDIO_ALERT_LAST_CLIPS,
            AUDIO_ALERT_TAIL_SILENCE_REPEATS
        );
    } else if woke_from_timer && boot_aod_time_alert {
        if alarm_loud_enabled_at_boot {
            play_loud_alert_signal!("scheduler timer wake loud sine chirp alert");
        } else {
            play_audio_sequence!(
                "scheduler timer wake clip",
                AUDIO_ALERT_FIRST_CLIPS,
                AUDIO_ALERT_MIDDLE_SILENCE_REPEATS,
                AUDIO_ALERT_LAST_CLIPS,
                AUDIO_ALERT_TAIL_SILENCE_REPEATS
            );
        }
    } else if woke_from_timer {
        println!("[AUDIO] Scheduler timer wake beep skipped; not near scheduled time");
    }
    #[cfg(not(feature = "audio"))]
    if STARTUP_BEEP_TEST || (woke_from_timer && boot_aod_time_alert) {
        println!("[AUDIO] Scheduler timer wake beep skipped; audio feature disabled");
    }

    // === Lazy radio/network ===
    #[cfg(feature = "ble")]
    let mut radio_tokens = Some((peripherals.WIFI, peripherals.BT));
    #[cfg(not(feature = "ble"))]
    let mut radio_tokens = Some(peripherals.WIFI);
    let mut wifi_controller: Option<esp_radio::wifi::WifiController<'static>> = None;
    let mut stack: Option<embassy_net::Stack<'static>> = None;
    #[cfg(feature = "ble")]
    let mut ble_connector: Option<esp_radio::ble::controller::BleConnector<'static>> = None;
    static RADIO: static_cell::StaticCell<esp_radio::Controller<'static>> =
        static_cell::StaticCell::new();
    static RESOURCES: static_cell::StaticCell<embassy_net::StackResources<3>> =
        static_cell::StaticCell::new();

    // Pre-fill STA credentials so "toggle WiFi" just flips a bit later.
    // Falls back to empty strings if WIFI_SSID / WIFI_PASS are not set at
    // compile time — WiFi simply won't connect but the watch boots fine.
    use esp_radio::wifi::{AuthMethod, ClientConfig, ModeConfig};
    let wifi_ssid = option_env!("WIFI_SSID").unwrap_or("");
    let wifi_pass = option_env!("WIFI_PASS").unwrap_or("");
    let wifi_has_creds = !wifi_ssid.is_empty();
    if !wifi_has_creds {
        println!("[WIFI] No SSID configured — WiFi disabled");
    }

    macro_rules! ensure_radio {
        () => {{
            if wifi_controller.is_none() {
                if let Some(radio_periphs) = radio_tokens.take() {
                    #[cfg(feature = "ble")]
                    let (wifi_periph, bt_periph) = radio_periphs;
                    #[cfg(not(feature = "ble"))]
                    let wifi_periph = radio_periphs;

                    println!("[RADIO] Lazy init radio stack...");
                    let radio_controller: &'static esp_radio::Controller<'static> =
                        RADIO.init(esp_radio::init().expect("esp-radio init failed"));

                    let wifi_config = esp_radio::wifi::Config::default()
                        .with_power_save_mode(esp_radio::wifi::PowerSaveMode::Maximum);
                    let (mut new_wifi_controller, wifi_interfaces) =
                        esp_radio::wifi::new(radio_controller, wifi_periph, wifi_config)
                            .expect("WiFi init failed");

                    let client_config = ClientConfig::default()
                        .with_ssid(alloc::string::String::from(wifi_ssid))
                        .with_password(alloc::string::String::from(wifi_pass))
                        .with_auth_method(if wifi_pass.is_empty() {
                            AuthMethod::None
                        } else {
                            AuthMethod::WpaWpa2Personal
                        });
                    let mode_config = ModeConfig::Client(client_config);
                    new_wifi_controller
                        .set_config(&mode_config)
                        .expect("WiFi config failed");

                    let resources = RESOURCES.init(embassy_net::StackResources::new());
                    let net_config = embassy_net::Config::dhcpv4(Default::default());
                    let (new_stack, runner) =
                        embassy_net::new(wifi_interfaces.sta, net_config, resources, 12345u64);
                    _spawner.spawn(net_task(runner)).ok();

                    wifi_controller = Some(new_wifi_controller);
                    stack = Some(new_stack);
                    #[cfg(feature = "ble")]
                    {
                        let new_ble_connector = esp_radio::ble::controller::BleConnector::new(
                            radio_controller,
                            bt_periph,
                            esp_radio::ble::Config::default(),
                        )
                        .expect("BLE init failed");
                        ble_connector = Some(new_ble_connector);
                    }
                    #[cfg(feature = "ble")]
                    println!("[RADIO] Ready (WiFi OFF, BLE OFF)");
                    #[cfg(not(feature = "ble"))]
                    println!("[RADIO] Ready (WiFi OFF)");
                } else {
                    println!("[RADIO] Init unavailable");
                }
            }

            wifi_controller.is_some()
        }};
    }

    // Keep the GPIO0 owner available so the deep-sleep path can drop this
    // Input and hand the same pin to RTC wake as BOOT.
    let mut boot_pin = peripherals.GPIO0;
    let mut boot_button = Input::new(
        boot_pin.reborrow(),
        InputConfig::default().with_pull(Pull::Up),
    );
    // If this boot was caused by the BOOT button, the pin may still be held low.
    // Do not treat it as a new sleep request until it has been released once.
    let mut boot_button_armed = !boot_button.is_low();
    let mut pwr_pin = peripherals.GPIO10;
    let mut pwr_button = Input::new(
        pwr_pin.reborrow(),
        InputConfig::default().with_pull(Pull::Up),
    );
    let mut pwr_button_armed = !pwr_button.is_low();
    println!("=== All systems GO! (Embassy async, WiFi OFF) ===");

    // Give the user a full interactive AOD window after the slower init
    // that now happens behind the already-visible first AOD frame.
    aod_entered_at = Instant::now();

    // === Event-driven async main loop ===
    //
    // The loop sleeps the CPU between iterations using `select` over:
    //   * a periodic timer whose period depends on the current app/screen state
    //   * the touch interrupt line (GPIO38, async falling edge)
    //   * the boot button line (GPIO0, async falling edge)
    //
    // Tick budgets per state (only consumed when nothing else wakes us):
    //   * screen off                : 30 s   (deep idle, just to refresh battery%)
    //   * watchface clock, gyro off : 1 s    (only the seconds digit changes)
    //   * watchface clock, gyro on  : 33 ms  (smooth gyro ball)
    //   * sensors page              : 100 ms (10 Hz IMU)
    //   * power page                : 1 s    (diagnostic — must not self-skew)
    //   * launcher / settings / mp3 : 100 ms
    //   * Snake / 2048 / Tetris / Maze / Flappy : 33 ms (~30 Hz, panel cap)
    //
    // A finger held on the screen forces 16 ms tick regardless of state.
    //
    // When the user touches the screen or presses BOOT, the select returns immediately
    // and we process the input. With this design, the CPU is parked >99% of the time
    // while sitting on the watchface.
    use embassy_futures::select::select4;

    let mut next_rtc = Instant::now();
    let mut next_battery = Instant::now();
    #[cfg(feature = "app-launcher")]
    let mut last_frame = Instant::now();
    let mut next_watchface_flush = Instant::now();
    // Radio state: we track both what the user *wants* and what the radio
    // actually is. They drift apart briefly during connect/disconnect.
    let mut wifi_on_request: bool = false; // user toggle
    let mut wifi_started: bool = false; // controller.start() called
    let mut wifi_connected: bool = false; // connect_async succeeded
    let mut ntp_synced: bool = false;
    let mut last_wifi_idle_check = Instant::now();
    // Request pending from a UI tap on the WiFi button.
    let mut wifi_toggle_request: bool = false;
    // BLE state
    #[cfg(feature = "ble")]
    let mut ble_on: bool = false;
    #[cfg(feature = "ble")]
    let mut ble_toggle_request: bool = false;
    // Power-down the IMU at boot — only enable when a consumer (gyro toggle, game, sensors page) needs it.
    let _ = imu.power_down();
    // IMU mode: 0 = off, 2 = accel + gyro.
    let mut imu_mode: u8 = 0;
    // Tracks the previous-iteration state of the FT3168 INT line.
    // We need this to keep polling touch.poll() ONCE more after the finger lifts,
    // otherwise we miss the swipe-end event and pages stay stuck mid-drag.
    let mut was_touching = false;

    loop {
        // Pick a tick budget based on current state. This is the MAX time we'll
        // sleep without any external wake source.
        let touch_held = touch_int.is_low();
        let button_held = boot_button.is_low();
        let pwr_held = pwr_button.is_low();

        let tick = if touch_held || button_held || pwr_held {
            // Something is currently being held: wake fast enough to track motion / detect
            // long-press, but no faster than necessary.
            Duration::from_millis(16) // ~60 Hz
        } else if screen_state == 0 {
            // Screen completely off: normally only wake every 10 min for housekeeping.
            Duration::from_secs(SCREEN_OFF_HOUSEKEEPING_SECS)
        } else if screen_state == 1 {
            // AOD mode: wake occasionally to check the off timeout or minute change.
            Duration::from_secs(AOD_DURATION_SECS.min(10))
        } else {
            match app_state {
                AppState::Watchface => match current_page {
                    Page::Aod => Duration::from_secs(AOD_DURATION_SECS.min(10)),
                    // Clock page: 1 Hz when gyro is off (only seconds change),
                    // 33 ms when gyro is on (smooth ball animation).
                    Page::Clock => {
                        if watchface.gyro_enabled {
                            Duration::from_millis(33)
                        } else {
                            Duration::from_secs(1)
                        }
                    }
                    Page::Sensors => Duration::from_millis(100), // 10 Hz IMU display
                    Page::System => Duration::from_secs(2),      // basically static
                    // Power page refreshes at 1 Hz — fast enough to see
                    // changes, slow enough not to skew the measurement.
                    Page::Power => Duration::from_secs(1),
                    Page::AlarmSettings => Duration::from_secs(2),
                    Page::QuickView => Duration::from_secs(2),
                },
                #[cfg(feature = "app-launcher")]
                AppState::Launcher => Duration::from_millis(100),
                #[cfg(feature = "settings")]
                AppState::Settings => Duration::from_millis(100),
                #[cfg(feature = "mp3-player")]
                AppState::Mp3Player => Duration::from_millis(100),
                #[cfg(feature = "smart-home")]
                AppState::SmartHome => Duration::from_millis(100),
                // Flappy previously ran at 8 ms (~125 Hz). The panel can't
                // even display that (VSync is ~33 ms) so the extra ticks
                // just burned CPU and DMA for no visible benefit.
                #[cfg(feature = "flappy")]
                AppState::Flappy => Duration::from_millis(33),
                #[cfg(feature = "snake")]
                AppState::Snake => Duration::from_millis(33),
                #[cfg(feature = "game-2048")]
                AppState::Game2048 => Duration::from_millis(33),
                #[cfg(feature = "tetris")]
                AppState::Tetris => Duration::from_millis(33),
                #[cfg(feature = "maze")]
                AppState::Maze => Duration::from_millis(33),
            }
        };

        // Sleep until the tick budget elapses OR a falling edge arrives on touch / boot button.
        // Notes:
        //   * If a pin is already low, wait_for_falling_edge will not fire (no edge to wait for),
        //     but the tick above is short (16 ms) so we still wake reactively.
        //   * The futures from esp-hal install GPIO interrupts on creation and remove them on
        //     drop, so the executor parks the CPU between events: this is the main power win.
        let _ = select4(
            Timer::after(tick),
            touch_int.wait_for_falling_edge(),
            boot_button.wait_for_falling_edge(),
            pwr_button.wait_for_falling_edge(),
        )
        .await;

        let now = Instant::now();
        #[cfg(feature = "app-launcher")]
        let dt_ms = {
            let elapsed_ms = (now - last_frame).as_millis() as u32;
            last_frame = now;
            elapsed_ms
        };

        let button_pressed = boot_button.is_low();
        let mut low_power_reason: Option<&'static str> = None;
        if button_pressed {
            if boot_button_armed {
                low_power_reason = Some("BOOT button");
            }
        } else {
            boot_button_armed = true;
        }
        let pwr_pressed = pwr_button.is_low();
        if pwr_pressed && pwr_button_armed {
            pwr_button_armed = false;
            last_interaction = now;
            app_state = AppState::Watchface;
            if current_page == Page::QuickView {
                current_page = Page::Aod;
                display.set_brightness(AOD_BRIGHTNESS);
                screen_state = 1;
                aod_entered_at = now;
                aod_last_minute = 99;
            } else if current_page == Page::AlarmSettings {
                current_page = Page::Aod;
                display.set_brightness(AOD_BRIGHTNESS);
                screen_state = 1;
                aod_entered_at = now;
                aod_last_minute = 99;
            } else {
                current_page = Page::QuickView;
                if !quick_view_loaded {
                    match rtc.get_alert_time() {
                        Ok(alert_time) => {
                            rtc_alert_time = alert_time;
                            sync_quick_view_alert_entry(
                                rtc_alert_time,
                                &mut quick_view_time_digits,
                                &mut quick_view_time_len,
                                &mut quick_view_show_set_key,
                            );
                        }
                        Err(_) => {
                            sync_quick_view_alert_entry(
                                rtc_alert_time,
                                &mut quick_view_time_digits,
                                &mut quick_view_time_len,
                                &mut quick_view_show_set_key,
                            );
                        }
                    }
                    quick_view_loaded = true;
                }
                if screen_state == 0 {
                    display.display_on();
                    Timer::after(Duration::from_millis(20)).await;
                }
                display.set_brightness(watchface.brightness);
                screen_state = 3;
            }
            watchface.force_redraw();
            page_dirty = true;
            next_watchface_flush = now;
        } else if !pwr_pressed {
            pwr_button_armed = true;
        }

        if low_power_reason.is_none()
            && current_page == Page::Aod
            && screen_state == 1
            && (now - aod_entered_at).as_secs() >= AOD_DURATION_SECS
        {
            low_power_reason = Some("AOD timeout");
        }

        if let Some(reason) = low_power_reason {
            println!("[POWER] Entering low power mode ({})", reason);

            let skip_current_alert_window = woke_from_timer && boot_aod_time_alert;
            let scheduled_wake = match rtc.get_time() {
                Ok(dt) => {
                    let now = rtc_to_time_manager_datetime(dt);
                    let minimum_duration = if skip_current_alert_window {
                        println!("[POWER] next wake: skipping current scheduler alert window");
                        Some(core::time::Duration::from_secs(AOD_ALERT_WINDOW_SECS))
                    } else {
                        None
                    };
                    let next_wake = next_wake_with_rtc_alert(now, rtc_alert_time, minimum_duration);

                    match next_wake {
                        Ok(Some(wake)) => {
                            println!("[POWER] next wake up in {}s", wake.duration.as_secs());
                            Some(wake.duration)
                        }
                        Ok(None) => {
                            println!("[POWER] next wake up: no scheduled times");
                            None
                        }
                        Err(err) => {
                            println!("[POWER] next wake up: schedule error {:?}", err);
                            None
                        }
                    }
                }
                Err(_) => {
                    println!("[POWER] next wake up: RTC read failed");
                    None
                }
            };

            if wifi_connected {
                if let Some(controller) = wifi_controller.as_mut() {
                    let _ = embassy_time::with_timeout(
                        Duration::from_secs(1),
                        controller.disconnect_async(),
                    )
                    .await;
                }
            }
            if wifi_started {
                if let Some(controller) = wifi_controller.as_mut() {
                    let _ = controller.stop();
                }
            }
            #[cfg(feature = "ble")]
            {
                if ble_on {
                    if let Some(connector) = ble_connector.as_mut() {
                        let _ = crate::peripherals::ble::stop_advertising(connector);
                    }
                }
            }

            #[cfg(feature = "audio")]
            {
                pa_en.set_low();
                if let Some(codec) = audio_codec.as_mut() {
                    let _ = codec.shutdown();
                }
            }
            if imu_initialized {
                let _ = imu.power_down();
            }
            let _ = touch.sleep();
            touch_rst.set_low();
            display.set_brightness(0x00);
            display.display_off();
            let _ = power.power_down_for_sleep();
            if scheduled_wake.is_some() {
                println!("[POWER] Deep sleep armed; wake sources: BOOT/GPIO0 low + timer");
            } else {
                println!("[POWER] Deep sleep armed; wake source: BOOT/GPIO0 low");
            }
            Timer::after(Duration::from_millis(50)).await;

            drop(boot_button);
            let boot_wake = Ext0WakeupSource::new(boot_pin, WakeupLevel::Low);
            if let Some(duration) = scheduled_wake {
                let timer_wake = TimerWakeupSource::new(duration);
                esp_rtc.sleep_deep(&[&boot_wake, &timer_wake]);
            } else {
                esp_rtc.sleep_deep(&[&boot_wake]);
            }
        }

        // === Sensors (gated by need + screen state) ===
        // IMU only when an interactive consumer needs it (gyro enabled, IMU-driven game, sensors page).
        // When screen is off OR no consumer needs it, we power-down the IMU completely
        // (CTRL7 = 0). The QMI8658's gyro alone draws ~1.5 mA so this is a meaningful win.
        let need_motion_imu = screen_state == 3
            && (watchface.gyro_enabled
                || app_needs_motion_imu(app_state)
                || (app_state == AppState::Watchface && current_page == Page::Sensors));
        if need_motion_imu && !imu_initialized {
            match imu.init() {
                Ok(true) => {
                    imu_initialized = true;
                    println!("[IMU] Lazy init OK");
                }
                Ok(false) => println!("[IMU] Lazy init: chip ID mismatch"),
                Err(_) => println!("[IMU] Lazy init failed"),
            }
        }
        let target_imu_mode = if need_motion_imu && imu_initialized {
            2
        } else {
            0
        };
        if target_imu_mode != imu_mode {
            match target_imu_mode {
                2 if imu_initialized => {
                    let _ = imu.power_up();
                }
                _ if imu_initialized => {
                    let _ = imu.power_down();
                }
                _ => {}
            }
            imu_mode = target_imu_mode;
        }
        if need_motion_imu && imu_initialized {
            if let Ok(a) = imu.read_accel() {
                accel = (a.x, a.y, a.z);
                watchface.update_accel(a.x, a.y, a.z);
            }
        }
        if need_motion_imu && imu_initialized {
            if let Ok(g) = imu.read_gyro() {
                gyro_data = (
                    (g.x * 10.0) as i16,
                    (g.y * 10.0) as i16,
                    (g.z * 10.0) as i16,
                );
            }
            if let Ok(t) = imu.read_temperature() {
                imu_temp = (t * 10.0) as i16;
            }
        }

        // RTC: 1 Hz update is enough for a clock display. Skip when screen is off OR in AOD
        // (AOD updates the RTC manually once per minute).
        if screen_state >= 2 && now >= next_rtc {
            if let Ok(dt) = rtc.get_time() {
                watchface.update_time(dt.hours, dt.minutes, dt.seconds);
                watchface.update_date(dt.day, dt.month, dt.year, dt.weekday);
            }
            next_rtc = now + Duration::from_secs(1);
        }

        // Battery: every 60 s normally, every 5 min when the screen is off
        // (we still check occasionally to track charge state and update on next wake).
        if now >= next_battery {
            if let Ok(pct) = power.get_battery_percent() {
                batt_pct = pct;
                batt_mv = power.get_battery_voltage().unwrap_or(0);
                charging = power.is_charging().unwrap_or(false);
                watchface.update_battery(batt_pct, batt_mv, charging);
            }
            next_battery = if screen_state == 0 {
                now + Duration::from_secs(600)
            } else {
                // Battery percent rarely changes faster than every few
                // minutes; polling every 60 s was pure I²C overhead.
                now + Duration::from_secs(180)
            };
        }

        // === Touch ===
        // Poll the I2C touch controller when:
        //   1. screen is on AND a finger is currently on the panel (INT low), OR
        //   2. screen is on AND the finger was on the panel last iteration (catches the
        //      lift/swipe-end event — without this, page swipes stay stuck mid-drag).
        // This keeps the bus quiet >99% of the time but never misses release events.
        let mut swipe_event = None;
        let mut tap_event = false;
        let int_low = touch_int.is_low();
        // AOD is also swipeable, so poll touch whenever the display is not fully off.
        let touch_active = screen_state != 0 && (int_low || was_touching);
        was_touching = int_low;
        if touch_active {
            if let Ok((point, event)) = touch.poll() {
                // Swipe handling for page navigation (only in Watchface mode)
                if app_state == AppState::Watchface {
                    if let Some(tp) = point {
                        last_touch_x = tp.x;
                        last_touch_y = tp.y;
                    }
                    if let Some(swipe) = event {
                        // Horizontal page swipes now switch immediately on release.
                        // The old live slide transition streamed partial frames while
                        // dragging, which could saturate the display bus and stutter.
                        let on_slider = current_page == Page::Clock
                            && WatchFace::brightness_from_tap(swipe.start_x, swipe.start_y)
                                .is_some();
                        match swipe.direction {
                            SwipeDirection::Left if !on_slider => {
                                current_page = current_page.next();
                                if current_page == Page::Aod {
                                    display.set_brightness(AOD_BRIGHTNESS);
                                    screen_state = 1;
                                    aod_entered_at = now;
                                    aod_last_minute = 99;
                                } else if screen_state < 3 {
                                    display.set_brightness(watchface.brightness);
                                    screen_state = 3;
                                }
                                page_dirty = true;
                                next_watchface_flush = now;
                            }
                            SwipeDirection::Right if !on_slider => {
                                current_page = current_page.prev();
                                if current_page == Page::Aod {
                                    display.set_brightness(AOD_BRIGHTNESS);
                                    screen_state = 1;
                                    aod_entered_at = now;
                                    aod_last_minute = 99;
                                } else if screen_state < 3 {
                                    display.set_brightness(watchface.brightness);
                                    screen_state = 3;
                                }
                                page_dirty = true;
                                next_watchface_flush = now;
                            }
                            _ => {
                                swipe_event = Some(swipe.direction);
                                tap_event = swipe.direction == SwipeDirection::Tap;
                            }
                        }
                    }
                } else {
                    // In app mode: track position + forward events
                    if let Some(tp) = point {
                        last_touch_x = tp.x;
                        last_touch_y = tp.y;
                    }
                    if let Some(swipe) = event {
                        swipe_event = Some(swipe.direction);
                        tap_event = swipe.direction == SwipeDirection::Tap;
                    }
                }
            }
        }
        // === Screen sleep/wake state machine ===
        // Levels:
        //   3 = full bright + interactive
        //   1 = AOD: minimal HH:MM, 80% brightness, 1 update/min, swipeable
        //   0 = full off (normally followed by ESP deep sleep)
        //
        // AOD timeout enters deep sleep earlier in the loop. Touch wakes only
        // while the display is still on; BOOT is reserved for low-power entry.
        let any_touch = touch_int.is_low();
        let aod_swipeable =
            app_state == AppState::Watchface && current_page == Page::Aod && screen_state == 1;
        if any_touch || swipe_event.is_some() || tap_event {
            last_interaction = now;
            if screen_state == 0 {
                // Wake the panel. If the selected page is AOD, keep it in AOD brightness;
                // swiping away from it promotes the display to interactive brightness.
                display.display_on();
                Timer::after(Duration::from_millis(20)).await;
                if app_state == AppState::Watchface && current_page == Page::Aod {
                    display.set_brightness(AOD_BRIGHTNESS);
                    screen_state = 1;
                    aod_entered_at = now;
                    aod_last_minute = 99;
                } else {
                    display.set_brightness(watchface.brightness);
                    screen_state = 3;
                }
                next_watchface_flush = now;
                if app_state == AppState::Watchface {
                    watchface.force_redraw();
                    page_dirty = true;
                }
            } else if screen_state < 3 && !aod_swipeable {
                // Wake up to full bright.
                display.set_brightness(watchface.brightness);
                screen_state = 3;
                next_watchface_flush = now;
                if app_state == AppState::Watchface {
                    watchface.force_redraw();
                    page_dirty = true;
                }
            }
        }
        let idle_secs = (now - last_interaction).as_secs();

        // === WiFi on/off state machine ===
        //
        // We take exactly one action per loop iteration so we don't block
        // the UI during a long-running connect_async(). User wants (wifi_on_request)
        // is driven by tapping the 'W' button on the watchface.
        //
        // On enable:  start() -> connect_async() -> set_power_save(MaxModem) -> NTP (once)
        // On disable: disconnect_async() -> stop()
        //
        // An "auto-off after 5 minutes idle" safety net is kept so that if
        // the user leaves WiFi enabled and wanders off, the radio drops on
        // its own. Turning it back on is manual — intentional.
        // Debounce the WiFi button: ignore rapid re-taps within 1 s.
        if wifi_toggle_request && (now - last_wifi_idle_check).as_millis() >= 1000 {
            wifi_on_request = !wifi_on_request;
            wifi_toggle_request = false;
            last_wifi_idle_check = now;
            println!(
                "[WIFI] User toggled → {}",
                if wifi_on_request { "ON" } else { "OFF" }
            );
        } else if wifi_toggle_request {
            wifi_toggle_request = false; // swallow the bounce
        }

        if wifi_on_request && !wifi_connected {
            if ensure_radio!() {
                if let Some(controller) = wifi_controller.as_mut() {
                    if !wifi_started {
                        if controller.start().is_ok() {
                            wifi_started = true;
                        }
                    }
                    if wifi_started {
                        // 15 s timeout — avoids blocking the UI forever if AP
                        // is unreachable or credentials are wrong.
                        match embassy_time::with_timeout(
                            Duration::from_secs(15),
                            controller.connect_async(),
                        )
                        .await
                        {
                            Ok(Ok(())) => {
                                println!("[WIFI] Connected (PS=MaxModem)");
                                wifi_connected = true;
                                watchface.wifi_connected = true;
                                watchface.force_redraw();
                                page_dirty = true;
                                // NTP sync only once per boot, after DHCP lands.
                                if !ntp_synced {
                                    if let Some(stack) = stack {
                                        println!("[WIFI] Waiting for DHCP IP...");
                                        for _ in 0..50 {
                                            if stack.config_v4().is_some() {
                                                break;
                                            }
                                            Timer::after(Duration::from_millis(100)).await;
                                        }
                                        if let Some(cfg) = stack.config_v4() {
                                            println!("[WIFI] IP acquired: {:?}", cfg.address);
                                            match ntp_sync(stack, &mut rtc).await {
                                                Ok(_) => {
                                                    ntp_synced = true;
                                                    println!("[NTP] synced");
                                                }
                                                Err(e) => println!("[NTP] sync failed: {:?}", e),
                                            }
                                        } else {
                                            println!("[WIFI] DHCP timeout");
                                        }
                                    }
                                }
                            }
                            _ => {
                                // Timeout or error — back off instead of hammering.
                                println!("[WIFI] Connect failed/timeout");
                                wifi_on_request = false;
                                watchface.wifi_connected = false;
                                watchface.force_redraw();
                                page_dirty = true;
                            }
                        }
                    }
                }
            } else {
                wifi_on_request = false;
                watchface.wifi_connected = false;
                watchface.force_redraw();
                page_dirty = true;
            }
            last_wifi_idle_check = now;
        }
        if !wifi_on_request && wifi_connected {
            if let Some(controller) = wifi_controller.as_mut() {
                let _ = controller.disconnect_async().await;
                let _ = controller.stop();
            }
            println!("[WIFI] Disconnected");
            wifi_connected = false;
            watchface.wifi_connected = false;
            wifi_started = false;
            watchface.force_redraw();
            page_dirty = true;
            last_wifi_idle_check = now;
        }
        // Safety net: WiFi left on, user idle 5 min → auto-off.
        if wifi_on_request && idle_secs >= 300 && (now - last_wifi_idle_check).as_secs() >= 60 {
            wifi_on_request = false;
            last_wifi_idle_check = now;
        }

        // === BLE state machine ===
        #[cfg(feature = "ble")]
        {
            if ble_toggle_request {
                ble_toggle_request = false;
                ble_on = !ble_on;
                if ble_on {
                    if ensure_radio!() {
                        if let Some(connector) = ble_connector.as_mut() {
                            match crate::peripherals::ble::start_advertising(connector) {
                                Ok(()) => println!("[BLE] Advertising started"),
                                Err(_) => {
                                    println!("[BLE] Failed to start advertising");
                                    ble_on = false;
                                }
                            }
                        } else {
                            println!("[BLE] Connector unavailable");
                            ble_on = false;
                        }
                    } else {
                        ble_on = false;
                    }
                } else {
                    if let Some(connector) = ble_connector.as_mut() {
                        let _ = crate::peripherals::ble::stop_advertising(connector);
                    }
                    println!("[BLE] Advertising stopped");
                }
                watchface.ble_on = ble_on;
                power_stats.ble_on = ble_on;
                watchface.force_redraw();
                page_dirty = true;
            }
        }

        // === AOD render path ===
        // In AOD we render *only* when the minute changes. Reads RTC, draws minimal
        // black-background HH:MM into the framebuffer, flushes once. Total work per minute:
        // ~1 RTC read + ~1 framebuffer fill + 1 DMA flush. The CPU sleeps the rest of the time.
        if current_page == Page::Aod && screen_state == 1 {
            if let Ok(dt) = rtc.get_time() {
                if dt.minutes != aod_last_minute {
                    aod_last_minute = dt.minutes;
                    watchface.update_time(dt.hours, dt.minutes, dt.seconds);
                    watchface.update_next_wake_time(aod_next_wake_time(dt, rtc_alert_time));
                    watchface.update_aod_time_alert_color(aod_scheduled_time_alert_color(
                        dt,
                        rtc_alert_time,
                    ));
                    if rtc_alert_due(dt, rtc_alert_time) {
                        let _ = rtc.disable_alert_time();
                        rtc_alert_time = None;
                        quick_view_loaded = false;
                        clear_quick_view_alert_entry(
                            &mut quick_view_time_digits,
                            &mut quick_view_time_len,
                            &mut quick_view_show_set_key,
                        );
                    }
                    if let Ok(pct) = power.get_battery_percent() {
                        watchface.update_battery(pct, batt_mv, charging);
                    }
                    let _ = watchface.render_aod(&mut fb);
                    fb.flush(&mut display);
                }
            }
            continue; // skip the normal app/render path
        }

        // When the screen is off, skip all rendering/flushing.
        // Keeps the QSPI bus idle so the CO5300 stays in a clean sleep state,
        // and the wake-up set_brightness/display_on commands always get through.
        if screen_state == 0 {
            continue;
        }

        // === App state machine ===
        match app_state {
            AppState::Watchface => {
                let mut need_flush = false;
                if page_dirty {
                    fb.clear_color(current_page.color());
                    match current_page {
                        Page::Aod => {
                            aod_last_minute = 99;
                        }
                        Page::Clock => {
                            watchface.force_redraw();
                        }
                        Page::System => {
                            let _ = pages::draw_system_page(&mut fb, batt_mv, batt_pct, charging);
                        }
                        Page::Power => {
                            // Rebuild stats snapshot, then render once.
                            // Subsequent frames will only redraw every
                            // ~1 s (see below) to keep the diagnostic
                            // itself cheap.
                            update_power_stats(
                                &mut power_stats,
                                screen_state,
                                imu_mode != 0,
                                wifi_connected,
                                wifi_on_request,
                                watchface.brightness,
                                batt_mv,
                                batt_pct,
                                charging,
                            );
                            let _ = power_page::draw_power_page(&mut fb, &power_stats);
                        }
                        Page::QuickView => {
                            let _ = pages::draw_quick_view(
                                &mut fb,
                                &quick_view_time_digits,
                                quick_view_show_set_key,
                                alarm_loud_enabled,
                            );
                        }
                        Page::AlarmSettings => {
                            let _ = pages::draw_alarm_settings_page(&mut fb, alarm_loud_enabled);
                        }
                        _ => {}
                    }
                    page_dirty = false;
                    need_flush = true;
                }
                match current_page {
                    Page::Aod => {
                        if let Ok(dt) = rtc.get_time() {
                            if dt.minutes != aod_last_minute {
                                aod_last_minute = dt.minutes;
                                watchface.update_time(dt.hours, dt.minutes, dt.seconds);
                                watchface
                                    .update_next_wake_time(aod_next_wake_time(dt, rtc_alert_time));
                                watchface.update_aod_time_alert_color(
                                    aod_scheduled_time_alert_color(dt, rtc_alert_time),
                                );
                                if rtc_alert_due(dt, rtc_alert_time) {
                                    let _ = rtc.disable_alert_time();
                                    rtc_alert_time = None;
                                    quick_view_loaded = false;
                                    clear_quick_view_alert_entry(
                                        &mut quick_view_time_digits,
                                        &mut quick_view_time_len,
                                        &mut quick_view_show_set_key,
                                    );
                                }
                                if let Ok(pct) = power.get_battery_percent() {
                                    watchface.update_battery(pct, batt_mv, charging);
                                }
                                let _ = watchface.render_aod(&mut fb);
                                need_flush = true;
                            }
                        }
                    }
                    Page::Clock => {
                        if watchface.needs_render() {
                            let _ = watchface.render(&mut fb);
                            need_flush = true;
                        }
                    }
                    Page::Sensors => {
                        let ax = (accel.0 * 100.0) as i16;
                        let ay = (accel.1 * 100.0) as i16;
                        let az = (accel.2 * 100.0) as i16;
                        fb.clear_color(current_page.color());
                        let _ = pages::draw_sensors_page(
                            &mut fb,
                            ax,
                            ay,
                            az,
                            gyro_data.0,
                            gyro_data.1,
                            gyro_data.2,
                            imu_temp,
                        );
                        need_flush = true;
                    }
                    Page::Power => {
                        if now >= next_watchface_flush {
                            update_power_stats(
                                &mut power_stats,
                                screen_state,
                                imu_mode != 0,
                                wifi_connected,
                                wifi_on_request,
                                watchface.brightness,
                                batt_mv,
                                batt_pct,
                                charging,
                            );
                            let _ = power_page::draw_power_page(&mut fb, &power_stats);
                            need_flush = true;
                            next_watchface_flush = now + Duration::from_secs(1);
                        }
                    }
                    Page::System | Page::AlarmSettings | Page::QuickView => {}
                }
                // Only flush if we actually drew something. The TE wait + 402 KB DMA
                // is by far the heaviest periodic operation in the firmware, so we
                // gate it strictly on dirtiness.
                if need_flush {
                    fb.flush_vsync(&mut display, &te_pin);
                    next_watchface_flush = now;
                }

                // Tap/touch dispatch on the Clock page.
                if current_page == Page::Clock {
                    // Brightness slider — responds to both taps and held
                    // drags so you can slide your finger along it.
                    if let Some(bri) = WatchFace::brightness_from_tap(last_touch_x, last_touch_y) {
                        if (touch_int.is_low() || tap_event) && bri != watchface.brightness {
                            watchface.brightness = bri;
                            display.set_brightness(bri);
                            watchface.force_redraw();
                            page_dirty = true;
                        }
                    } else if tap_event {
                        #[cfg(any(feature = "ble", feature = "app-launcher"))]
                        let mut handled_tap = false;
                        #[cfg(not(any(feature = "ble", feature = "app-launcher")))]
                        let handled_tap = false;

                        #[cfg(feature = "ble")]
                        {
                            // BLE toggle
                            if WatchFace::is_ble_zone(last_touch_x, last_touch_y) {
                                ble_toggle_request = true;
                                watchface.force_redraw();
                                page_dirty = true;
                                handled_tap = true;
                            }
                        }

                        // WiFi toggle
                        if !handled_tap && WatchFace::is_wifi_zone(last_touch_x, last_touch_y) {
                            wifi_toggle_request = true;
                            watchface.force_redraw();
                            page_dirty = true;
                        // CPU frequency cycle (live DVFS)
                        } else if !handled_tap && WatchFace::is_cpu_zone(last_touch_x, last_touch_y)
                        {
                            watchface.cycle_cpu();
                            let actual =
                                crate::peripherals::cpu_clock::set_cpu_mhz(watchface.cpu_mhz);
                            watchface.cpu_mhz = actual;
                            power_stats.cpu_mhz = actual;
                            println!("CPU freq: {}MHz (live)", actual);
                            watchface.force_redraw();
                            page_dirty = true;
                        } else {
                            #[cfg(feature = "app-launcher")]
                            {
                                if !handled_tap
                                    && WatchFace::is_apps_zone(last_touch_x, last_touch_y)
                                {
                                    app_state = AppState::Launcher;
                                    handled_tap = true;
                                }
                            }
                            // Gyro toggle
                            if !handled_tap && WatchFace::is_gyro_zone(last_touch_y) {
                                let enabled = watchface.toggle_gyro();
                                println!("Gyro: {}", if enabled { "ON" } else { "OFF" });
                            }
                        }
                    }
                }

                // QuickView numpad input. Digits fill HHMM from left to right;
                // the colon is rendered by the view and is not entered.
                if current_page == Page::QuickView && tap_event {
                    if pages::quick_view_loud_toggle_at(last_touch_x, last_touch_y) {
                        alarm_loud_enabled = !alarm_loud_enabled;
                        let _ = rtc.set_loud_flag(alarm_loud_enabled);
                        page_dirty = true;
                        next_watchface_flush = now;
                    } else if let Some(key) = pages::quick_view_key_at(last_touch_x, last_touch_y) {
                        match key {
                            pages::QuickViewKey::Digit(digit) => {
                                if quick_view_time_len < quick_view_time_digits.len() {
                                    quick_view_time_digits[quick_view_time_len] = Some(digit);
                                    quick_view_time_len += 1;
                                    page_dirty = true;
                                }
                            }
                            pages::QuickViewKey::Clear => {
                                clear_quick_view_alert_entry(
                                    &mut quick_view_time_digits,
                                    &mut quick_view_time_len,
                                    &mut quick_view_show_set_key,
                                );
                                page_dirty = true;
                            }
                            pages::QuickViewKey::Set if quick_view_show_set_key => {
                                let had_input = quick_view_time_len > 0;
                                for digit in quick_view_time_digits.iter_mut() {
                                    if digit.is_none() {
                                        *digit = Some(0);
                                    }
                                }
                                let hours = quick_view_time_digits[0].unwrap_or(0) * 10
                                    + quick_view_time_digits[1].unwrap_or(0);
                                let minutes = quick_view_time_digits[2].unwrap_or(0) * 10
                                    + quick_view_time_digits[3].unwrap_or(0);
                                if had_input {
                                    let _ = rtc.set_alert_time(hours, minutes);
                                    rtc_alert_time = Some((hours, minutes));
                                } else {
                                    let _ = rtc.disable_alert_time();
                                    rtc_alert_time = None;
                                    clear_quick_view_alert_entry(
                                        &mut quick_view_time_digits,
                                        &mut quick_view_time_len,
                                        &mut quick_view_show_set_key,
                                    );
                                }
                                if had_input {
                                    quick_view_time_len = quick_view_time_digits.len();
                                    quick_view_show_set_key = false;
                                }
                                page_dirty = true;
                            }
                            pages::QuickViewKey::Set => {}
                        }
                    }
                }
                // Reboot button on Power page
                if current_page == Page::Power && tap_event {
                    if power_page::is_reboot_zone(last_touch_x, last_touch_y) {
                        println!("REBOOT requested");
                        esp_hal::system::software_reset();
                    }
                }

                // Swipe up on Clock → launcher
                #[cfg(feature = "app-launcher")]
                {
                    if let Some(SwipeDirection::Up) = swipe_event {
                        if current_page == Page::Clock {
                            app_state = AppState::Launcher;
                        }
                    }
                }
            }

            #[cfg(feature = "snake")]
            AppState::Snake => {
                #[cfg(feature = "audio")]
                let prev_score = snake_game.score();
                let input = AppInput {
                    touch: None,
                    swipe: swipe_event,
                    tap: tap_event,
                    accel,
                    dt_ms: dt_ms.max(1),
                };
                match snake_game.update(&input) {
                    AppResult::Continue => {
                        if snake_game.stepped() {
                            snake_game.render(&mut fb);
                            fb.flush(&mut display);
                            // Beep when food eaten via I2S DMA
                            #[cfg(feature = "audio")]
                            {
                                if snake_game.score() > prev_score {
                                    play_audio_sequence!("snake food clip", 1, 0, 0, 0);
                                }
                            }
                        }
                    }
                    AppResult::Exit => {
                        app_state = AppState::Watchface;
                        watchface.force_redraw();
                        page_dirty = true;
                    }
                }
            }

            #[cfg(feature = "app-launcher")]
            AppState::Launcher => {
                // Track touch Y for tap detection
                if let Ok((point, _)) = touch.poll() {
                    if let Some(tp) = point {
                        last_touch_y = tp.y;
                    }
                }
                if let Some(new_state) = launcher.update(swipe_event, tap_event, last_touch_y) {
                    app_state = new_state;
                    match app_state {
                        #[cfg(feature = "snake")]
                        AppState::Snake => snake_game.setup(),
                        #[cfg(feature = "game-2048")]
                        AppState::Game2048 => {
                            game_2048.setup();
                            game_2048.render(&mut fb);
                            fb.flush(&mut display);
                        }
                        #[cfg(feature = "tetris")]
                        AppState::Tetris => tetris_game.setup(),
                        #[cfg(feature = "flappy")]
                        AppState::Flappy => flappy_game.setup(),
                        #[cfg(feature = "maze")]
                        AppState::Maze => maze_game.setup(),
                        #[cfg(feature = "mp3-player")]
                        AppState::Mp3Player => {
                            ensure_mp3_scan!();
                            mp3_player.setup();
                            mp3_player.set_track_count(mp3_files.len());
                            if !mp3_files.is_empty() {
                                mp3_player.set_track_name(&mp3_files[0]);
                            }
                        }
                        #[cfg(feature = "smart-home")]
                        AppState::SmartHome => smarthome_app.setup(),
                        #[cfg(feature = "settings")]
                        AppState::Settings => {}
                        AppState::Watchface => {
                            watchface.force_redraw();
                            page_dirty = true;
                        }
                        _ => {}
                    }
                } else {
                    launcher.render(&mut fb);
                    fb.flush(&mut display);
                }
            }

            #[cfg(feature = "game-2048")]
            AppState::Game2048 => {
                let input = AppInput {
                    touch: None,
                    swipe: swipe_event,
                    tap: tap_event,
                    accel,
                    dt_ms: dt_ms.max(1),
                };
                game_2048.update(&input);
                // Only render on input (swipe moves tiles)
                if swipe_event.is_some() {
                    game_2048.render(&mut fb);
                    fb.flush_vsync(&mut display, &te_pin);
                }
            }

            #[cfg(feature = "tetris")]
            AppState::Tetris => {
                let input = AppInput {
                    touch: None,
                    swipe: swipe_event,
                    tap: tap_event,
                    accel,
                    dt_ms: dt_ms.max(1),
                };
                tetris_game.update(&input);
                if tetris_game.stepped() || swipe_event.is_some() || tap_event {
                    tetris_game.render(&mut fb);
                    fb.flush_vsync(&mut display, &te_pin);
                }
            }

            #[cfg(feature = "flappy")]
            AppState::Flappy => {
                // Touch via GPIO38 (instant)
                let touch_down = touch_int.is_low();
                let fake_touch = if touch_down {
                    Some(crate::peripherals::touch::TouchPoint {
                        x: 200,
                        y: 250,
                        fingers: 1,
                    })
                } else {
                    None
                };
                let input = AppInput {
                    touch: fake_touch,
                    swipe: swipe_event,
                    tap: tap_event,
                    accel,
                    dt_ms: dt_ms.max(1),
                };
                flappy_game.update(&input);
                // Double-buffered render: draw to fb, swap+flush with VSync
                flappy_game.render(&mut fb);
                if now >= next_watchface_flush {
                    fb.swap_and_flush(&mut display, &te_pin);
                    next_watchface_flush = now + Duration::from_millis(33);
                }
            }

            #[cfg(feature = "maze")]
            AppState::Maze => {
                let input = AppInput {
                    touch: None,
                    swipe: swipe_event,
                    tap: tap_event,
                    accel,
                    dt_ms: dt_ms.max(1),
                };
                maze_game.update(&input);
                // Maze renders at 30fps (IMU continuous)
                if now >= next_watchface_flush {
                    maze_game.render(&mut fb);
                    fb.flush_vsync(&mut display, &te_pin);
                    next_watchface_flush = now + Duration::from_millis(33);
                }
            }

            #[cfg(feature = "smart-home")]
            AppState::SmartHome => {
                let input = AppInput {
                    touch: None,
                    swipe: swipe_event,
                    tap: tap_event,
                    accel,
                    dt_ms: dt_ms.max(1),
                };
                smarthome_app.update(&input);
                // TODO: when get_pending_request() returns a URL, send HTTP request via embassy-net
                // For now just show the UI
                smarthome_app.render(&mut fb);
                if now >= next_watchface_flush {
                    fb.flush_vsync(&mut display, &te_pin);
                    next_watchface_flush = now + Duration::from_millis(100);
                }
            }

            #[cfg(feature = "mp3-player")]
            AppState::Mp3Player => {
                let input = AppInput {
                    touch: None,
                    swipe: swipe_event,
                    tap: tap_event,
                    accel,
                    dt_ms: dt_ms.max(1),
                };
                mp3_player.update(&input);
                mp3_player.render(&mut fb);
                if now >= next_watchface_flush {
                    fb.flush_vsync(&mut display, &te_pin);
                    next_watchface_flush = now + Duration::from_millis(200);
                }
            }

            #[cfg(feature = "settings")]
            AppState::Settings => {
                settings_app.update(dt_ms.max(1));
                // For T9: detect touch down via GPIO38 for rapid multi-tap
                if tap_event {
                    settings_app.handle_tap(last_touch_x, last_touch_y);
                }
                // Also read live touch position for keyboard area
                if let Ok((Some(tp), _)) = touch.poll() {
                    last_touch_x = tp.x;
                    last_touch_y = tp.y;
                }
                settings_app.render(&mut fb);
                if now >= next_watchface_flush {
                    fb.flush_vsync(&mut display, &te_pin);
                    next_watchface_flush = now + Duration::from_millis(50);
                }
            }
        }
    }
}
