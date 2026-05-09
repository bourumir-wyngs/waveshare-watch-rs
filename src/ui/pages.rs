// Multi-page system
// Pages: AOD | Clock | Sensors | System Info | Power | Quick view

use embedded_graphics::mono_font::ascii::FONT_10X20;
use embedded_graphics::mono_font::MonoTextStyle;
use embedded_graphics::pixelcolor::Rgb565;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{Line, PrimitiveStyle};
use embedded_graphics::text::{Alignment, Text};

use crate::board;
const W: u16 = board::LCD_WIDTH;
const H: u16 = board::LCD_HEIGHT;

#[derive(Clone, Copy, PartialEq)]
pub enum Page {
    Aod = 0,
    Clock = 1,
    Sensors = 2,
    System = 3,
    Power = 4,
    QuickView = 5,
}

impl Page {
    pub fn count() -> usize {
        6
    }

    pub fn next(self) -> Self {
        match self {
            Page::Aod => Page::Clock,
            Page::Clock => Page::Sensors,
            Page::Sensors => Page::System,
            Page::System => Page::Power,
            Page::Power => Page::Aod,
            Page::QuickView => Page::Clock,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            Page::Aod => Page::Power,
            Page::Clock => Page::Aod,
            Page::Sensors => Page::Clock,
            Page::System => Page::Sensors,
            Page::Power => Page::System,
            Page::QuickView => Page::Power,
        }
    }

    pub fn color(self) -> Rgb565 {
        // All pages use pure black AMOLED background for battery savings
        Rgb565::BLACK
    }

    pub fn name(self) -> &'static str {
        match self {
            Page::Aod => "AOD",
            Page::Clock => "CLOCK",
            Page::Sensors => "SENSORS",
            Page::System => "SYSTEM",
            Page::Power => "POWER",
            Page::QuickView => "QUICK",
        }
    }
}

/// Quick-view keypad key.
#[derive(Clone, Copy, PartialEq)]
pub enum QuickViewKey {
    Digit(u8),
    Set,
    Clear,
}

/// Map a touch point to a quick-view keypad key.
pub fn quick_view_key_at(x: u16, y: u16) -> Option<QuickViewKey> {
    let cell_w = W / 3;
    let cell_h = H / 5;
    let row = y / cell_h;
    let col = (x / cell_w).min(2);

    match (row, col) {
        (1, 0) => Some(QuickViewKey::Digit(7)),
        (1, 1) => Some(QuickViewKey::Digit(8)),
        (1, 2) => Some(QuickViewKey::Digit(9)),
        (2, 0) => Some(QuickViewKey::Digit(4)),
        (2, 1) => Some(QuickViewKey::Digit(5)),
        (2, 2) => Some(QuickViewKey::Digit(6)),
        (3, 0) => Some(QuickViewKey::Digit(1)),
        (3, 1) => Some(QuickViewKey::Digit(2)),
        (3, 2) => Some(QuickViewKey::Digit(3)),
        (4, 0) => Some(QuickViewKey::Digit(0)),
        (4, 1) => Some(QuickViewKey::Set),
        (4, 2) => Some(QuickViewKey::Clear),
        _ => None,
    }
}

/// Draw the second-button quick view.
pub fn draw_quick_view<D: DrawTarget<Color = Rgb565>>(
    display: &mut D,
    time_digits: &[Option<u8>; 4],
) -> Result<(), D::Error> {
    let cx = W as i32 / 2;
    let rust = Rgb565::new(31, 18, 0);
    let grid_line = PrimitiveStyle::with_stroke(rust, 2);

    let cell_w = W as i32 / 3;
    let cell_h = H as i32 / 5;

    for x in [cell_w, cell_w * 2] {
        Line::new(Point::new(x, cell_h), Point::new(x, H as i32 - 1))
            .into_styled(grid_line)
            .draw(display)?;
    }

    for y in [cell_h, cell_h * 2, cell_h * 3, cell_h * 4] {
        Line::new(Point::new(0, y), Point::new(W as i32 - 1, y))
            .into_styled(grid_line)
            .draw(display)?;
    }

    draw_time_indicator(
        display,
        time_digits,
        Point::new(cx, cell_h / 2),
        Rgb565::WHITE,
    )?;

    let labels = [
        ["7", "8", "9"],
        ["4", "5", "6"],
        ["1", "2", "3"],
        ["0", "S", "C"],
    ];
    for (row_idx, row) in labels.iter().enumerate() {
        let y = cell_h * (row_idx as i32 + 1) + cell_h / 2;
        for (col_idx, label) in row.iter().enumerate() {
            let x = cell_w * col_idx as i32 + cell_w / 2;
            draw_large_key_label(display, label, Point::new(x, y), rust)?;
        }
    }

    Ok(())
}

fn draw_time_indicator<D: DrawTarget<Color = Rgb565>>(
    display: &mut D,
    digits: &[Option<u8>; 4],
    center: Point,
    color: Rgb565,
) -> Result<(), D::Error> {
    let positions = [center.x - 102, center.x - 52, center.x + 52, center.x + 102];
    for (idx, digit) in digits.iter().enumerate() {
        if let Some(digit) = digit {
            let label = match digit {
                0 => "0",
                1 => "1",
                2 => "2",
                3 => "3",
                4 => "4",
                5 => "5",
                6 => "6",
                7 => "7",
                8 => "8",
                9 => "9",
                _ => "",
            };
            draw_large_key_label(display, label, Point::new(positions[idx], center.y), color)?;
        }
    }

    let colon_style = PrimitiveStyle::with_stroke(color, 6);
    Line::new(
        Point::new(center.x, center.y - 18),
        Point::new(center.x, center.y - 12),
    )
    .into_styled(colon_style)
    .draw(display)?;
    Line::new(
        Point::new(center.x, center.y + 12),
        Point::new(center.x, center.y + 18),
    )
    .into_styled(colon_style)
    .draw(display)?;

    Ok(())
}

fn draw_large_key_label<D: DrawTarget<Color = Rgb565>>(
    display: &mut D,
    label: &str,
    center: Point,
    color: Rgb565,
) -> Result<(), D::Error> {
    let style = PrimitiveStyle::with_stroke(color, 6);
    let x0 = center.x - 17;
    let x1 = center.x + 17;
    let y0 = center.y - 29;
    let ym = center.y;
    let y1 = center.y + 29;

    let segments = match label {
        "0" => [true, true, true, false, true, true, true],
        "1" => [false, false, true, false, false, true, false],
        "2" => [true, false, true, true, true, false, true],
        "3" => [true, false, true, true, false, true, true],
        "4" => [false, true, true, true, false, true, false],
        "5" | "S" => [true, true, false, true, false, true, true],
        "6" => [true, true, false, true, true, true, true],
        "7" => [true, false, true, false, false, true, false],
        "8" => [true, true, true, true, true, true, true],
        "9" => [true, true, true, true, false, true, true],
        "C" => [true, true, false, false, true, false, true],
        _ => [false; 7],
    };

    let lines = [
        (Point::new(x0, y0), Point::new(x1, y0)),
        (Point::new(x0, y0), Point::new(x0, ym)),
        (Point::new(x1, y0), Point::new(x1, ym)),
        (Point::new(x0, ym), Point::new(x1, ym)),
        (Point::new(x0, ym), Point::new(x0, y1)),
        (Point::new(x1, ym), Point::new(x1, y1)),
        (Point::new(x0, y1), Point::new(x1, y1)),
    ];

    for (on, (start, end)) in segments.iter().zip(lines.iter()) {
        if *on {
            Line::new(*start, *end).into_styled(style).draw(display)?;
        }
    }

    Ok(())
}

/// Draw the sensors page content.
pub fn draw_sensors_page<D: DrawTarget<Color = Rgb565>>(
    display: &mut D,
    ax: i16,
    ay: i16,
    az: i16,
    gx: i16,
    gy: i16,
    gz: i16,
    temp: i16,
) -> Result<(), D::Error> {
    let cx = W as i32 / 2;
    let cyan = MonoTextStyle::new(&FONT_10X20, Rgb565::CYAN);
    let white = MonoTextStyle::new(&FONT_10X20, Rgb565::WHITE);
    let green = MonoTextStyle::new(&FONT_10X20, Rgb565::GREEN);
    let yellow = MonoTextStyle::new(&FONT_10X20, Rgb565::YELLOW);
    let dim = MonoTextStyle::new(&FONT_10X20, Rgb565::CSS_GRAY);

    Text::with_alignment("SENSORS", Point::new(cx, 40), cyan, Alignment::Center).draw(display)?;

    Text::with_alignment("Accelerometer", Point::new(cx, 90), dim, Alignment::Center)
        .draw(display)?;

    let mut buf = [0u8; 16];
    fmt_axis(&mut buf, b'X', ax);
    Text::with_alignment(
        core::str::from_utf8(&buf[..8]).unwrap_or(""),
        Point::new(cx, 120),
        green,
        Alignment::Center,
    )
    .draw(display)?;
    fmt_axis(&mut buf, b'Y', ay);
    Text::with_alignment(
        core::str::from_utf8(&buf[..8]).unwrap_or(""),
        Point::new(cx, 150),
        green,
        Alignment::Center,
    )
    .draw(display)?;
    fmt_axis(&mut buf, b'Z', az);
    Text::with_alignment(
        core::str::from_utf8(&buf[..8]).unwrap_or(""),
        Point::new(cx, 180),
        green,
        Alignment::Center,
    )
    .draw(display)?;

    Text::with_alignment("Gyroscope", Point::new(cx, 230), dim, Alignment::Center).draw(display)?;

    fmt_axis(&mut buf, b'X', gx);
    Text::with_alignment(
        core::str::from_utf8(&buf[..8]).unwrap_or(""),
        Point::new(cx, 260),
        yellow,
        Alignment::Center,
    )
    .draw(display)?;
    fmt_axis(&mut buf, b'Y', gy);
    Text::with_alignment(
        core::str::from_utf8(&buf[..8]).unwrap_or(""),
        Point::new(cx, 290),
        yellow,
        Alignment::Center,
    )
    .draw(display)?;
    fmt_axis(&mut buf, b'Z', gz);
    Text::with_alignment(
        core::str::from_utf8(&buf[..8]).unwrap_or(""),
        Point::new(cx, 320),
        yellow,
        Alignment::Center,
    )
    .draw(display)?;

    let mut tbuf = [0u8; 10];
    let ts = fmt_temp(&mut tbuf, temp);
    Text::with_alignment(ts, Point::new(cx, 380), white, Alignment::Center).draw(display)?;

    Ok(())
}

/// Draw the system info page.
pub fn draw_system_page<D: DrawTarget<Color = Rgb565>>(
    display: &mut D,
    batt_mv: u16,
    batt_pct: u8,
    charging: bool,
) -> Result<(), D::Error> {
    let cx = W as i32 / 2;
    let cyan = MonoTextStyle::new(&FONT_10X20, Rgb565::CYAN);
    let white = MonoTextStyle::new(&FONT_10X20, Rgb565::WHITE);
    let dim = MonoTextStyle::new(&FONT_10X20, Rgb565::CSS_GRAY);
    let green = MonoTextStyle::new(&FONT_10X20, Rgb565::GREEN);

    Text::with_alignment("SYSTEM", Point::new(cx, 40), cyan, Alignment::Center).draw(display)?;

    Text::with_alignment(
        "ESP32-S3 160MHz",
        Point::new(cx, 90),
        white,
        Alignment::Center,
    )
    .draw(display)?;
    Text::with_alignment("8MB PSRAM", Point::new(cx, 120), dim, Alignment::Center).draw(display)?;
    Text::with_alignment("32MB Flash", Point::new(cx, 150), dim, Alignment::Center)
        .draw(display)?;
    Text::with_alignment(
        "QSPI 80MHz DMA",
        Point::new(cx, 180),
        dim,
        Alignment::Center,
    )
    .draw(display)?;

    Text::with_alignment("Firmware", Point::new(cx, 230), cyan, Alignment::Center).draw(display)?;
    Text::with_alignment(
        "waveshare-watch",
        Point::new(cx, 260),
        white,
        Alignment::Center,
    )
    .draw(display)?;
    Text::with_alignment("v0.3 Rust", Point::new(cx, 290), green, Alignment::Center)
        .draw(display)?;
    Text::with_alignment("~110KB binary", Point::new(cx, 320), dim, Alignment::Center)
        .draw(display)?;

    let chg_str = if charging {
        "USB: Connected"
    } else {
        "USB: Battery"
    };
    Text::with_alignment(chg_str, Point::new(cx, 370), white, Alignment::Center).draw(display)?;

    let mut buf = [0u8; 12];
    let vs = fmt_mv(&mut buf, batt_mv);
    Text::with_alignment(vs, Point::new(cx, 400), dim, Alignment::Center).draw(display)?;

    Ok(())
}

fn fmt_axis(buf: &mut [u8; 16], label: u8, val: i16) {
    buf[0] = label;
    buf[1] = b':';
    buf[2] = b' ';
    if val < 0 {
        buf[3] = b'-';
    } else {
        buf[3] = b'+';
    }
    let v = val.unsigned_abs();
    buf[4] = b'0' + (v / 100) as u8;
    buf[5] = b'.';
    buf[6] = b'0' + ((v / 10) % 10) as u8;
    buf[7] = b'0' + (v % 10) as u8;
}

fn fmt_temp<'a>(buf: &'a mut [u8; 10], temp_c10: i16) -> &'a str {
    let mut p = 0;
    if temp_c10 < 0 {
        buf[p] = b'-';
        p += 1;
    }
    let v = temp_c10.unsigned_abs();
    buf[p] = b'0' + (v / 100) as u8;
    p += 1;
    buf[p] = b'0' + ((v / 10) % 10) as u8;
    p += 1;
    buf[p] = b'.';
    p += 1;
    buf[p] = b'0' + (v % 10) as u8;
    p += 1;
    buf[p] = b'C';
    p += 1;
    core::str::from_utf8(&buf[..p]).unwrap_or("??C")
}

fn fmt_mv<'a>(buf: &'a mut [u8; 12], mv: u16) -> &'a str {
    let mut p = 0;
    if mv >= 1000 {
        buf[p] = b'0' + (mv / 1000) as u8;
        p += 1;
    }
    buf[p] = b'0' + ((mv / 100) % 10) as u8;
    p += 1;
    buf[p] = b'0' + ((mv / 10) % 10) as u8;
    p += 1;
    buf[p] = b'0' + (mv % 10) as u8;
    p += 1;
    for &c in b"mV" {
        buf[p] = c;
        p += 1;
    }
    core::str::from_utf8(&buf[..p]).unwrap_or("????mV")
}
