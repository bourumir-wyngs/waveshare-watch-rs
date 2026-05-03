use embedded_graphics::pixelcolor::Rgb565;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{PrimitiveStyle, Rectangle};

pub const WIDTH: i32 = 64;
pub const HEIGHT: i32 = 43;

const FERRIS_RLE: &[u8] = include_bytes!("../logos/ferris_aod_64x43.rgb565.rle");

pub fn draw<D: DrawTarget<Color = Rgb565>>(d: &mut D, top_left: Point) -> Result<(), D::Error> {
    let mut pixel_index = 0i32;

    for run in FERRIS_RLE.chunks_exact(3) {
        let raw = u16::from_be_bytes([run[0], run[1]]);
        let mut len = run[2] as i32;

        if raw == 0 {
            pixel_index += len;
            continue;
        }

        let color = rgb565_from_raw(raw);
        while len > 0 {
            let x = pixel_index % WIDTH;
            let y = pixel_index / WIDTH;
            let draw_len = len.min(WIDTH - x);

            Rectangle::new(
                Point::new(top_left.x + x, top_left.y + y),
                Size::new(draw_len as u32, 1),
            )
            .into_styled(PrimitiveStyle::with_fill(color))
            .draw(d)?;

            pixel_index += draw_len;
            len -= draw_len;
        }
    }

    debug_assert_eq!(pixel_index, WIDTH * HEIGHT);
    Ok(())
}

fn rgb565_from_raw(raw: u16) -> Rgb565 {
    Rgb565::new(
        ((raw >> 11) & 0x1f) as u8,
        ((raw >> 5) & 0x3f) as u8,
        (raw & 0x1f) as u8,
    )
}
