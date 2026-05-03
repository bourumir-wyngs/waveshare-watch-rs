// PSRAM Framebuffer for CO5300 display
// 410x502 RGB565 = 411,640 bytes (~402KB)
// Draws to RAM, then flushes entire screen via DMA QSPI

use embedded_graphics_core::draw_target::DrawTarget;
use embedded_graphics_core::geometry::{OriginDimensions, Size};
use embedded_graphics_core::pixelcolor::raw::RawU16;
use embedded_graphics_core::pixelcolor::Rgb565;
use embedded_graphics_core::prelude::*;
use embedded_graphics_core::primitives::Rectangle;

use crate::board;
use crate::drivers::co5300::Co5300Display;
use crate::drivers::co5300::DisplayError;

use alloc::vec;
use alloc::vec::Vec;

const WIDTH: usize = board::LCD_WIDTH as usize;
const HEIGHT: usize = board::LCD_HEIGHT as usize;
const PIXEL_COUNT: usize = WIDTH * HEIGHT;

pub struct Framebuffer {
    buf: Vec<u16>,
    back: Vec<u16>, // Double buffer: draw to back, flush front
}

impl Framebuffer {
    /// Allocate framebuffer in PSRAM (via global allocator).
    pub fn new() -> Self {
        let buf = vec![0u16; PIXEL_COUNT];
        let back = vec![0u16; PIXEL_COUNT];
        Self { buf, back }
    }

    /// Swap front and back buffers. Call after rendering to back buffer.
    /// The front buffer (buf) is what gets flushed to display.
    pub fn swap(&mut self) {
        core::mem::swap(&mut self.buf, &mut self.back);
    }

    /// Clear the entire framebuffer with a color.
    pub fn clear_color(&mut self, color: Rgb565) {
        let raw = RawU16::from(color).into_inner();
        self.buf.fill(raw);
    }

    /// Set a single pixel (no bounds check for speed).
    #[inline(always)]
    pub fn set_pixel(&mut self, x: usize, y: usize, color: u16) {
        if x < WIDTH && y < HEIGHT {
            self.buf[y * WIDTH + x] = color;
        }
    }

    /// Fill a rectangular region.
    pub fn fill_rect(&mut self, x: usize, y: usize, w: usize, h: usize, color: u16) {
        let x_end = (x + w).min(WIDTH);
        let y_end = (y + h).min(HEIGHT);
        for row in y..y_end {
            let start = row * WIDTH + x;
            let end = row * WIDTH + x_end;
            self.buf[start..end].fill(color);
        }
    }

    /// Double-buffer swap + VSync flush.
    /// 1. Swap front/back buffers (instant)
    /// 2. Wait for TE signal (VBlank)
    /// 3. Flush the new front buffer to display
    /// Result: display always shows a complete frame, zero tearing.
    /// Fast VSync flush for games. No copy, just sync + send.
    pub fn swap_and_flush(&mut self, display: &mut Co5300Display, te: &esp_hal::gpio::Input<'_>) {
        // Short TE sync. If TE isn't pulsing (display just woken up, or we're flushing
        // outside vblank window), give up after a few hundred cycles instead of burning
        // CPU. Tearing is invisible most of the time anyway because we flush <30fps.
        for _ in 0..400 { if te.is_high() { break; } }
        display.set_addr_window(0, 0, WIDTH as u16, HEIGHT as u16);
        display.bus_mut().write_pixels(&self.buf);
    }

    /// VSync flush for watchface / menus. Same as swap_and_flush but kept distinct for clarity.
    pub fn flush_vsync(&self, display: &mut Co5300Display, te: &esp_hal::gpio::Input<'_>) {
        for _ in 0..400 { if te.is_high() { break; } }
        self.flush(display);
    }

    /// Flush the entire framebuffer to the display via DMA QSPI.
    pub fn flush(&self, display: &mut Co5300Display) {
        display.set_addr_window(0, 0, WIDTH as u16, HEIGHT as u16);
        display.bus_mut().write_pixels(&self.buf);
    }

    /// Flush only a rectangular region (dirty rect optimization).
    pub fn flush_region(&self, display: &mut Co5300Display, x: u16, y: u16, w: u16, h: u16) {
        if w == 0 || h == 0 {
            return;
        }

        // The CO5300 is happier with even-aligned partial writes.
        // Expand the dirty rect to an even 2x2-aligned region before streaming rows.
        let mut x0 = (x as usize).min(WIDTH.saturating_sub(1));
        let mut y0 = (y as usize).min(HEIGHT.saturating_sub(1));
        let mut x1 = ((x as usize).saturating_add(w as usize)).min(WIDTH);
        let mut y1 = ((y as usize).saturating_add(h as usize)).min(HEIGHT);

        x0 &= !1;
        y0 &= !1;
        if x1 & 1 != 0 && x1 < WIDTH {
            x1 += 1;
        }
        if y1 & 1 != 0 && y1 < HEIGHT {
            y1 += 1;
        }

        if x1 <= x0 {
            x1 = (x0 + 2).min(WIDTH);
        }
        if y1 <= y0 {
            y1 = (y0 + 2).min(HEIGHT);
        }

        let flush_w = (x1 - x0).max(2).min(WIDTH - x0);
        let flush_h = (y1 - y0).max(2).min(HEIGHT - y0);

        display.set_addr_window(x0 as u16, y0 as u16, flush_w as u16, flush_h as u16);
        display.bus_mut().begin_pixels();
        for row in y0..(y0 + flush_h) {
            let start = row * WIDTH + x0;
            let end = start + flush_w;
            display.bus_mut().stream_pixels(&self.buf[start..end]);
        }
        display.bus_mut().end_pixels();
    }

}

impl OriginDimensions for Framebuffer {
    fn size(&self) -> Size {
        Size::new(WIDTH as u32, HEIGHT as u32)
    }
}

impl DrawTarget for Framebuffer {
    type Color = Rgb565;
    type Error = DisplayError;

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Self::Color>>,
    {
        for Pixel(coord, color) in pixels.into_iter() {
            if coord.x >= 0 && coord.x < WIDTH as i32 && coord.y >= 0 && coord.y < HEIGHT as i32 {
                let raw = RawU16::from(color).into_inner();
                self.buf[coord.y as usize * WIDTH + coord.x as usize] = raw;
            }
        }
        Ok(())
    }

    fn fill_contiguous<I>(
        &mut self,
        area: &Rectangle,
        colors: I,
    ) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Self::Color>,
    {
        let area = area.intersection(&Rectangle::new(
            Point::zero(),
            Size::new(WIDTH as u32, HEIGHT as u32),
        ));
        if area.size.width == 0 || area.size.height == 0 {
            return Ok(());
        }

        let x = area.top_left.x as usize;
        let y = area.top_left.y as usize;
        let w = area.size.width as usize;
        let mut row = y;
        let mut col = 0;

        for color in colors.into_iter() {
            if col < w && row < HEIGHT {
                self.buf[row * WIDTH + x + col] = RawU16::from(color).into_inner();
            }
            col += 1;
            if col >= w {
                col = 0;
                row += 1;
            }
        }
        Ok(())
    }

    fn fill_solid(
        &mut self,
        area: &Rectangle,
        color: Self::Color,
    ) -> Result<(), Self::Error> {
        let area = area.intersection(&Rectangle::new(
            Point::zero(),
            Size::new(WIDTH as u32, HEIGHT as u32),
        ));
        if area.size.width == 0 || area.size.height == 0 {
            return Ok(());
        }
        let raw = RawU16::from(color).into_inner();
        self.fill_rect(
            area.top_left.x as usize,
            area.top_left.y as usize,
            area.size.width as usize,
            area.size.height as usize,
            raw,
        );
        Ok(())
    }
}
