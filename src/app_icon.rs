pub const DEFAULT_ICON_SIZE: u32 = 256;
const DESIGN_SIZE: u32 = 256;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IconBitmap {
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

pub fn render_icon(size: u32) -> IconBitmap {
    let size = size.max(16);
    let mut rgba = vec![0; (size * size * 4) as usize];

    let background = [30, 31, 27, 255];
    let frame = [82, 84, 88, 255];
    let board_edge = [51, 52, 56, 255];
    let light_square = [206, 186, 150, 255];
    let dark_square = [123, 92, 67, 255];
    let accent = [146, 178, 110, 255];
    let piece_fill = [248, 246, 240, 255];
    let piece_outline = [18, 20, 19, 255];

    fill_rounded_rect(
        &mut rgba,
        size,
        scale(0, size),
        scale(0, size),
        scale(256, size),
        scale(256, size),
        scale(56, size),
        background,
    );
    fill_rounded_rect(
        &mut rgba,
        size,
        scale(12, size),
        scale(12, size),
        scale(244, size),
        scale(244, size),
        scale(48, size),
        frame,
    );
    fill_rounded_rect(
        &mut rgba,
        size,
        scale(26, size),
        scale(26, size),
        scale(230, size),
        scale(230, size),
        scale(42, size),
        board_edge,
    );

    fill_rect(
        &mut rgba,
        size,
        scale(42, size),
        scale(42, size),
        scale(128, size),
        scale(128, size),
        light_square,
    );
    fill_rect(
        &mut rgba,
        size,
        scale(128, size),
        scale(42, size),
        scale(214, size),
        scale(128, size),
        dark_square,
    );
    fill_rect(
        &mut rgba,
        size,
        scale(42, size),
        scale(128, size),
        scale(128, size),
        scale(214, size),
        dark_square,
    );
    fill_rect(
        &mut rgba,
        size,
        scale(128, size),
        scale(128, size),
        scale(214, size),
        scale(214, size),
        light_square,
    );
    fill_rect(
        &mut rgba,
        size,
        scale(42, size),
        scale(118, size),
        scale(214, size),
        scale(138, size),
        accent,
    );

    draw_rook_icon(
        &mut rgba,
        size,
        scale(2, size),
        scale(3, size),
        piece_outline,
    );
    draw_rook_icon(&mut rgba, size, 0, 0, piece_fill);

    IconBitmap {
        rgba,
        width: size,
        height: size,
    }
}

fn draw_rook_icon(rgba: &mut [u8], width: u32, dx: i32, dy: i32, color: [u8; 4]) {
    fill_rounded_rect(rgba, width, dx + scale(78, width), dy + scale(182, width), dx + scale(178, width), dy + scale(208, width), scale(8, width), color);
    fill_rounded_rect(rgba, width, dx + scale(90, width), dy + scale(78, width), dx + scale(166, width), dy + scale(190, width), scale(10, width), color);
    fill_rounded_rect(rgba, width, dx + scale(80, width), dy + scale(70, width), dx + scale(176, width), dy + scale(94, width), scale(8, width), color);
    fill_rounded_rect(rgba, width, dx + scale(82, width), dy + scale(48, width), dx + scale(104, width), dy + scale(82, width), scale(5, width), color);
    fill_rounded_rect(rgba, width, dx + scale(117, width), dy + scale(40, width), dx + scale(139, width), dy + scale(82, width), scale(5, width), color);
    fill_rounded_rect(rgba, width, dx + scale(152, width), dy + scale(48, width), dx + scale(174, width), dy + scale(82, width), scale(5, width), color);
}

fn fill_rect(
    rgba: &mut [u8],
    width: u32,
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
    color: [u8; 4],
) {
    fill_rounded_rect(rgba, width, left, top, right, bottom, 0, color);
}

fn fill_rounded_rect(
    rgba: &mut [u8],
    width: u32,
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
    radius: i32,
    color: [u8; 4],
) {
    let height = (rgba.len() / 4) as i32 / width as i32;
    for y in top.max(0)..bottom.min(height) {
        for x in left.max(0)..right.min(width as i32) {
            if point_in_rounded_rect(x, y, left, top, right, bottom, radius) {
                let index = ((y as u32 * width + x as u32) * 4) as usize;
                rgba[index..index + 4].copy_from_slice(&color);
            }
        }
    }
}

fn point_in_rounded_rect(
    x: i32,
    y: i32,
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
    radius: i32,
) -> bool {
    if radius <= 0 {
        return x >= left && x < right && y >= top && y < bottom;
    }

    let inner_left = left + radius;
    let inner_right = right - radius - 1;
    let inner_top = top + radius;
    let inner_bottom = bottom - radius - 1;
    let nearest_x = if inner_left <= inner_right {
        x.clamp(inner_left, inner_right)
    } else {
        (left + right - 1) / 2
    };
    let nearest_y = if inner_top <= inner_bottom {
        y.clamp(inner_top, inner_bottom)
    } else {
        (top + bottom - 1) / 2
    };
    let dx = x - nearest_x;
    let dy = y - nearest_y;

    dx * dx + dy * dy <= radius * radius
}

fn scale(value: i32, size: u32) -> i32 {
    ((i64::from(value) * i64::from(size) + i64::from(DESIGN_SIZE / 2)) / i64::from(DESIGN_SIZE))
        as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rendered_icon_has_expected_dimensions() {
        let icon = render_icon(DEFAULT_ICON_SIZE);

        assert_eq!(icon.width, DEFAULT_ICON_SIZE);
        assert_eq!(icon.height, DEFAULT_ICON_SIZE);
        assert_eq!(icon.rgba.len(), (DEFAULT_ICON_SIZE * DEFAULT_ICON_SIZE * 4) as usize);
    }

    #[test]
    fn rendered_icon_scales_to_large_sizes() {
        let icon = render_icon(1024);

        assert_eq!(icon.width, 1024);
        assert_eq!(icon.height, 1024);
        assert_eq!(icon.rgba.len(), 1024 * 1024 * 4);
    }
}
