use std::path::Path;

use anyhow::{Context, Result};
use image::{GenericImageView, RgbaImage};

use crate::index::{is_frameish, Index, IndexGlyph, BASELINE, FRAME_WIDTH};
use crate::point::Point;

/// Each glyph has an invisible edge of `r=0xff g=0x00 b=0xff a=0x00` pixels along the right and
/// bottom forming a sort of "frame".
///
/// There may be a cyan `r=0x00 g=0xff b=0xff a=0x00` pixel along the right edge that indicates
/// where the baseline of that glyph should be. This pixel is optional, the entire right edge can
/// be pink like the entire bottom frame.
///
/// . = visible pixel of glyph in texture
/// p = pink frame pixel
/// c = cyan frame pixel
///
/// ........p
/// ........p
/// ........c
/// ........p
/// ppppppppp
///
/// The methods here assume the image origin (0, 0) is top left.
#[derive(Debug, Clone, Copy)]
struct Glyph {
    /// `tl` is the xy pixel coordinates of the Glyph's top-left visible pixel within the
    /// entire atlas.
    tl: Point,
    /// `br` like `tr` but for the bottom-right visible pixel (not the frame pixel)
    br: Point,
    /// distance from the bottom frame to the cyan pixel, zero if no cyan pixel. (would be 2 in the
    /// example in the struct docstring above.)
    descent: u32,
}

impl Glyph {
    fn x(&self) -> u32 {
        self.tl.x
    }

    fn y(&self) -> u32 {
        self.tl.y
    }

    fn w(&self) -> u32 {
        self.br.x.saturating_sub(self.tl.x).saturating_add(1)
    }

    fn h(&self) -> u32 {
        self.br.y.saturating_sub(self.tl.y).saturating_add(1)
    }
}

pub struct Atlas<'i> {
    buf: &'i RgbaImage,
    rows: Vec<Vec<Glyph>>,
}

#[derive(Debug, thiserror::Error)]
enum NoGlyph {
    #[error("no visible glyph, just frame")]
    JustFrame { point: Point },
    #[error("reached image edge")]
    End,
}

impl<'i> Atlas<'i> {
    pub fn len(&self) -> usize {
        self.rows.iter().map(|row| row.len()).sum()
    }

    pub fn row_count(&self) -> usize {
        self.rows.len()
    }

    pub fn from_image(buf: &'i RgbaImage) -> Self {
        let mut point = Point { x: 0, y: 0 };

        let mut rows = Vec::<Vec<Glyph>>::default();

        loop {
            let mut row = Vec::<Glyph>::default();

            while point.x < buf.width() {
                match Glyph::from_image(buf, point) {
                    Ok(glyph) => {
                        point.x = glyph.br.x + FRAME_WIDTH + 1;
                        row.push(glyph);
                    }
                    Err(NoGlyph::End) | Err(NoGlyph::JustFrame { .. }) => break,
                }
            }

            if let Some(glyph) = row.first() {
                point = Point {
                    x: 0,
                    y: glyph.br.y + FRAME_WIDTH + 1,
                };
                rows.push(row);
            } else {
                break;
            }
        }

        Atlas { rows, buf }
    }

    pub fn save_images<P: AsRef<Path>>(&self, outdir: P) -> Result<Index> {
        let mut i = 0;
        let rows = self
            .rows
            .iter()
            .map(|row| {
                row.iter()
                    .map(|glyph| {
                        let path = format!("{i:03}.png");
                        i += 1;

                        let filepath = outdir.as_ref().join(&path);

                        self.buf
                            .view(glyph.x(), glyph.y(), glyph.w(), glyph.h())
                            .to_image()
                            .save(&filepath)
                            .with_context(|| format!("save {}", filepath.display()))?;

                        Ok(IndexGlyph {
                            path,
                            descent: glyph.descent,
                        })
                    })
                    .collect::<Result<Vec<_>>>()
            })
            .collect::<Result<Vec<_>>>()?;

        Ok(Index { rows })
    }
}

impl Glyph {
    fn from_image(buf: &RgbaImage, tl: Point) -> Result<Glyph, NoGlyph> {
        let (_, frame_bl) = tl
            .scan_y(buf)
            .find(|(&pixel, _at)| is_frameish(pixel))
            .ok_or(NoGlyph::End)?;

        if frame_bl == tl {
            return Err(NoGlyph::JustFrame { point: tl });
        }

        let (_, frame_tr) = tl
            .next_x()
            .ok_or(NoGlyph::End)?
            .scan_x(buf)
            .find(|(&pixel, _at)| is_frameish(pixel))
            .ok_or(NoGlyph::End)?;

        let descent = frame_tr
            .y_counter()
            .take_while(|&Point { y, .. }| y < frame_bl.y)
            .map_while(|at| at.get_pixel_at(buf))
            .find(|(&pixel, _at)| pixel == BASELINE)
            /* no subtraction overflow because take_while() above */
            .map(|(_, Point { y, .. })| frame_bl.y - y)
            .unwrap_or(0);

        let br = Point {
            x: frame_tr.x - FRAME_WIDTH,
            y: frame_bl.y - FRAME_WIDTH,
        };

        Ok(Glyph { tl, br, descent })
    }
}
