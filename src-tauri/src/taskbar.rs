//! Previous / play-pause / next buttons under the window preview when the mouse is over Miu's
//! taskbar button (a Windows thumbnail toolbar, like iTunes). Clicks reach the page as a
//! `taskbar-button` event, which presses the matching player button, so they behave exactly the same.

/// The glyphs on the buttons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Glyph {
    Prev,
    Play,
    Pause,
    Next,
}

/// Button ids (as Windows reports them in `THBN_CLICKED`) and what the page should press.
pub const BUTTONS: [(u32, &str); 3] = [(1, "prev"), (2, "play"), (3, "next")];

pub fn button_name(id: u32) -> Option<&'static str> {
    BUTTONS.iter().find(|(i, _)| *i == id).map(|(_, n)| *n)
}

/// A `size`×`size` RGBA image of `glyph` in `rgb`, anti-aliased (4×4 samples per pixel).
pub fn glyph(glyph: Glyph, size: u32, rgb: [u8; 3]) -> Vec<u8> {
    const SAMPLES: u32 = 4;
    let mut out = Vec::with_capacity((size * size * 4) as usize);
    for py in 0..size {
        for px in 0..size {
            let mut hits = 0;
            for sy in 0..SAMPLES {
                for sx in 0..SAMPLES {
                    let x = (px as f32 + (sx as f32 + 0.5) / SAMPLES as f32) / size as f32;
                    let y = (py as f32 + (sy as f32 + 0.5) / SAMPLES as f32) / size as f32;
                    if inside(glyph, x, y) {
                        hits += 1;
                    }
                }
            }
            let a = (hits * 255 / (SAMPLES * SAMPLES)) as u8;
            let pre = |c: u8| (c as u32 * a as u32 / 255) as u8;
            out.extend([pre(rgb[0]), pre(rgb[1]), pre(rgb[2]), a]);
        }
    }
    out
}

/// Whether the point (x, y) in the unit square is part of the glyph.
fn inside(glyph: Glyph, x: f32, y: f32) -> bool {
    let rect = |x0: f32, x1: f32| (x0..x1).contains(&x) && (0.2..0.8).contains(&y);
    match glyph {
        Glyph::Play => triangle((x, y), (0.3, 0.2), (0.3, 0.8), (0.8, 0.5)),
        Glyph::Pause => rect(0.28, 0.42) || rect(0.58, 0.72),
        Glyph::Prev => rect(0.2, 0.3) || triangle((x, y), (0.8, 0.2), (0.8, 0.8), (0.3, 0.5)),
        Glyph::Next => inside(Glyph::Prev, 1.0 - x, y),
    }
}

fn triangle(p: (f32, f32), a: (f32, f32), b: (f32, f32), c: (f32, f32)) -> bool {
    let side = |p: (f32, f32), q: (f32, f32), r: (f32, f32)| (q.0 - p.0) * (r.1 - p.1) - (q.1 - p.1) * (r.0 - p.0);
    let (d1, d2, d3) = (side(a, b, p), side(b, c, p), side(c, a, p));
    !((d1 < 0.0 || d2 < 0.0 || d3 < 0.0) && (d1 > 0.0 || d2 > 0.0 || d3 > 0.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    const WHITE: [u8; 3] = [255, 255, 255];

    fn alpha(img: &[u8], size: u32, x: f32, y: f32) -> u8 {
        let (px, py) = ((x * size as f32) as u32, (y * size as f32) as u32);
        img[((py * size + px) * 4 + 3) as usize]
    }

    #[test]
    fn a_glyph_is_a_square_rgba_image_with_clear_corners() {
        for g in [Glyph::Prev, Glyph::Play, Glyph::Pause, Glyph::Next] {
            let img = glyph(g, 20, WHITE);
            assert_eq!(img.len(), 20 * 20 * 4);
            assert_eq!(alpha(&img, 20, 0.0, 0.0), 0, "{g:?}");
            assert_eq!(alpha(&img, 20, 0.97, 0.97), 0, "{g:?}");
        }
    }

    #[test]
    fn play_points_right_and_pause_has_two_bars() {
        let play = glyph(Glyph::Play, 32, WHITE);
        assert_eq!(alpha(&play, 32, 0.4, 0.5), 255);
        assert_eq!(alpha(&play, 32, 0.4, 0.22), 0, "narrow towards the top on the right");
        assert_eq!(alpha(&play, 32, 0.9, 0.5), 0);
        let pause = glyph(Glyph::Pause, 32, WHITE);
        assert_eq!(alpha(&pause, 32, 0.35, 0.5), 255);
        assert_eq!(alpha(&pause, 32, 0.65, 0.5), 255);
        assert_eq!(alpha(&pause, 32, 0.5, 0.5), 0, "gap between the bars");
    }

    #[test]
    fn next_is_prev_mirrored() {
        let (prev, next) = (glyph(Glyph::Prev, 24, WHITE), glyph(Glyph::Next, 24, WHITE));
        for y in 0..24 {
            for x in 0..24 {
                let p = ((y * 24 + x) * 4 + 3) as usize;
                let n = ((y * 24 + (23 - x)) * 4 + 3) as usize;
                assert_eq!(prev[p], next[n], "({x},{y})");
            }
        }
    }

    #[test]
    fn glyphs_use_the_given_colour_with_premultiplied_edges() {
        let img = glyph(Glyph::Play, 16, [32, 32, 32]);
        let solid = img.chunks(4).find(|p| p[3] == 255).unwrap();
        assert_eq!(&solid[..3], &[32, 32, 32]);
        let edge = img.chunks(4).find(|p| p[3] > 0 && p[3] < 255).expect("anti-aliased edge");
        assert!(edge[0] <= 32, "colour is premultiplied by alpha for Windows icons");
    }

    #[test]
    fn clicks_map_to_player_buttons() {
        assert_eq!(button_name(1), Some("prev"));
        assert_eq!(button_name(2), Some("play"));
        assert_eq!(button_name(3), Some("next"));
        assert_eq!(button_name(9), None);
    }
}
