use std::fmt::{self, Write};
use std::path::Path;

use anyhow::{Context, Result};
use image::{Rgba, RgbaImage};

use crate::point::Point;

pub const FRAME: Rgba<u8> = Rgba([255, 0, 255, 0]) /* hot pink */;
pub const BASELINE: Rgba<u8> = Rgba([0, 255, 255, 0]) /* teal */;
pub const FRAME_WIDTH: u32 = 1;

pub fn is_frameish(rgba: Rgba<u8>) -> bool {
    rgba == FRAME || rgba == BASELINE
}

#[derive(Debug, Clone)]
pub struct Index {
    pub rows: Vec<Vec<IndexGlyph>>,
}

impl Index {
    pub fn len(&self) -> usize {
        self.rows.iter().map(|row| row.len()).sum()
    }
}

#[derive(Debug, Clone)]
pub struct IndexGlyph {
    pub path: String,
    pub descent: u32,
}

#[derive(Debug)]
pub struct LoadedIndex {
    pub rows: Vec<Vec<LoadedGlyph>>,
}

#[derive(Debug)]
pub struct LoadedGlyph {
    pub glyph: IndexGlyph,
    pub image: RgbaImage,
}

impl Index {
    pub fn to_html(&self) -> String {
        let mut s = String::from(
            r#"
<!DOCTYPE html>
<head>
<meta name="viewport" content="width=device-width,initial-scale=1">
<script>
/* Browsers seem to scale up images based on the desktop's pixel ratio thing.
 * So try to reverse that so the images display accurately...? */
document.querySelector(':root')
	.style
	.setProperty('--ppiUnscale', 1.0 / window.devicePixelRatio)
</script>
<style>
body
  { background: #282828 }
[data-atlas]
  { display: flex; grid-gap: 1px }
[data-atlas='']
  { flex-direction: column;
    transform-origin: top left;
    scale: var(--ppiUnscale) }
</style>
</head>
"#,
        );
        let _ = writeln!(s, "<div data-atlas>");

        for row in self.rows.iter() {
            let _ = writeln!(s, "  <div data-atlas=row>");

            for glyph in row.iter() {
                let _ = writeln!(s, "    {}", glyph.to_img_tag());
            }

            let _ = writeln!(s, "  </div>");
        }

        let _ = writeln!(s, "</div>");
        s
    }

    pub fn patch_html(&self, html: &str) -> Result<(usize, String)> {
        let patch;

        let mut dom = tl::parse(html, tl::ParserOptions::default())?;

        let mut images = dom
            .query_selector_unchecked("img[src]")
            .filter_map(|img| {
                let src = dom.tag(img)?.attribute_value("src")?;
                Some((src, img))
            })
            .collect::<Vec<(&str, tl::NodeHandle)>>();
        images.sort_by(|(a, _), (b, _)| a.cmp(b));

        patch = self
            .rows
            .iter()
            .flat_map(|row| row.iter())
            .filter_map(|glyph| {
                /* Look for an <img> with the same src attribute.
                 * If found, render the replacement glyph (which should be the same except maybe a
                 * different descent value?) */
                images
                    .binary_search_by(|&(src, _)| src.cmp(glyph.path.as_str()))
                    .ok()
                    .and_then(|index| images.get(index).cloned())
                    .map(|(_, node)| (node, glyph.to_img_tag().to_string()))
            })
            .collect::<Vec<(tl::NodeHandle, String)>>();

        for (node, glyph) in patch.iter() {
            if let Some(node) = dom.node_mut(*node) {
                *node = tl::Node::Raw(tl::Bytes::from(glyph.as_str()));
            }
        }

        Ok((patch.len(), dom.outer_html()))
    }

    pub fn from_html(s: &str) -> Result<Self> {
        let dom = tl::parse(s, tl::ParserOptions::default())?;
        let parser = dom.parser();
        dom.query_selector_unchecked("[data-atlas=row]")
            .filter_map(|row| dom.tag(row))
            .map(|row| {
                row.query_selector(parser, "img[src]")
                    .expect("img[src]")
                    .filter_map(|img| dom.tag(img))
                    .map(IndexGlyph::from_img_tag)
                    .collect::<Result<Vec<_>>>()
            })
            .collect::<Result<Vec<_>>>()
            .map(|rows| Index { rows })
    }

    pub fn load_images<P: AsRef<Path>>(self, root: P) -> Result<LoadedIndex> {
        let rows = self
            .rows
            .into_iter()
            .map(|row| {
                row.into_iter()
                    .map(|glyph| {
                        let glyph_path = root.as_ref().join(&glyph.path);
                        let image = image::open(&glyph_path)
                            .with_context(|| format!("open {}", glyph_path.display()))?
                            .into_rgba8();
                        Ok(LoadedGlyph { glyph, image })
                    })
                    .collect::<Result<Vec<_>>>()
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(LoadedIndex { rows })
    }
}

impl IndexGlyph {
    fn to_img_tag(&self) -> impl fmt::Display + '_ {
        return IndexGlyphToImgTag(self);

        struct IndexGlyphToImgTag<'a>(&'a IndexGlyph);

        impl<'a> fmt::Display for IndexGlyphToImgTag<'a> {
            fn fmt(&self, s: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(s, "<img src='")?;
                write_html_encoded_attribute_value(s, &self.0.path)?;
                if self.0.descent > 0 {
                    write!(s, "' data-descent={}>", self.0.descent)
                } else {
                    write!(s, "'>")
                }
            }
        }
    }

    fn from_img_tag(img: &tl::HTMLTag<'_>) -> Result<Self> {
        let path = img
            .attribute_value("src")
            .context("img missing src attribute?")?
            .to_owned();

        let descent = img
            .attribute_value("data-descent")
            .map(|s| {
                s.parse()
                    .with_context(|| format!("expected non-negative integer, found: {s}"))
            })
            .transpose()?
            .unwrap_or(0);

        Ok(IndexGlyph { path, descent })
    }
}

impl LoadedIndex {
    pub fn to_atlas_image(&self, (width, height): (u32, u32)) -> Result<RgbaImage> {
        let width = match width {
            0 => self.widest_row_width().unwrap_or(0),
            _ => width,
        };
        let row_heights = self.row_heights();
        let height = match height {
            0 => row_heights.iter().sum(),
            _ => height,
        };

        if width == 0 || height == 0 {
            return Err(anyhow::anyhow!(
                "cowardly refusing to make an image with dimensions {width}x{height}"
            ));
        }

        let mut atlas = RgbaImage::new(width, height);

        atlas.pixels_mut().for_each(|p| *p = FRAME);

        row_heights.into_iter().zip(self.rows.iter()).try_fold(
            Point { x: 0, y: 0 },
            |point, (row_height, row)| {
                row.iter().try_fold(point, |point, loaded| {
                    let LoadedGlyph { image, glyph } = loaded;
                    copy_glyph_to_atlas(&mut atlas, point, image, glyph.descent);
                    image
                        .width()
                        .checked_add(FRAME_WIDTH)
                        .and_then(|dx| point.x.checked_add(dx))
                        .map(|x| Point { x, y: point.y })
                });
                point.checked_add(Point {
                    x: 0,
                    y: row_height,
                })
            },
        );

        Ok(atlas)
    }

    fn widest_row_width(&self) -> Option<u32> {
        self.rows
            .iter()
            .map(|row| {
                row.iter()
                    .map(|loaded| loaded.image.width() + FRAME_WIDTH)
                    .sum()
            })
            .max()
    }

    fn row_heights(&self) -> Vec<u32> {
        self.rows
            .iter()
            .map(|row| {
                row.iter()
                    .map(|loaded| loaded.image.height() + FRAME_WIDTH)
                    .max()
                    .unwrap_or(0)
            })
            .collect::<Vec<_>>()
    }
}

fn copy_glyph_to_atlas(
    atlas: &mut RgbaImage,
    topleft: Point,
    glyph_image: &RgbaImage,
    descent: u32,
) {
    /* apparently this will skip over regions of glyph_image that aren't inside of the atlas, for
     * if the image dimensions or `point` is wacky */
    image::imageops::replace(atlas, glyph_image, topleft.x as i64, topleft.y as i64);

    if descent > 0 {
        topleft
            .checked_add(Point {
                x: glyph_image.width(),
                y: glyph_image.height().saturating_sub(descent),
            })
            .and_then(|p| atlas.get_pixel_mut_checked(p.x, p.y))
            .map(|p| *p = BASELINE);
    }
}

fn write_html_encoded_attribute_value<W, S>(w: &mut W, s: S) -> fmt::Result
where
    W: Write,
    S: AsRef<str>,
{
    for c in s.as_ref().chars() {
        if let Some(enc) = html_encoded_char(c) {
            write!(w, "{}", enc)?;
        } else {
            write!(w, "{}", c)?;
        }
    }

    Ok(())
}

fn html_encoded_char(c: char) -> Option<&'static str> {
    Some(match c {
        '&' => "&amp;",
        '<' => "&lt;",
        '>' => "&gt;",
        '"' => "&quot;",
        '\'' => "&#x27;",
        _ => return None,
    })
}

use tl::queryselector::{iterable::QueryIterable, QuerySelectorIterator};

trait Dom<'buf>: Sized + QueryIterable<'buf> {
    fn node(&self, handle: tl::NodeHandle) -> Option<&tl::Node<'buf>>;

    fn node_mut(&mut self, handle: tl::NodeHandle) -> Option<&mut tl::Node<'buf>>;

    fn tag(&self, handle: tl::NodeHandle) -> Option<&tl::HTMLTag<'buf>> {
        self.node(handle).and_then(tl::Node::as_tag)
    }

    fn query_selector_unchecked<'dom>(
        &'dom self,
        selector: &'static str,
    ) -> QuerySelectorIterator<'buf, 'dom, Self>;
}

impl<'buf> Dom<'buf> for tl::VDom<'buf> {
    fn node(&self, handle: tl::NodeHandle) -> Option<&tl::Node<'buf>> {
        handle.get(self.parser())
    }

    fn node_mut(&mut self, handle: tl::NodeHandle) -> Option<&mut tl::Node<'buf>> {
        handle.get_mut(self.parser_mut())
    }

    fn query_selector_unchecked<'dom>(
        &'dom self,
        selector: &'static str,
    ) -> QuerySelectorIterator<'buf, 'dom, Self> {
        self.query_selector(selector).expect(selector)
    }
}

trait DomTag<'buf> {
    fn attribute_value(&self, attribute: &'buf str) -> Option<&str>;
}

impl<'buf> DomTag<'buf> for tl::HTMLTag<'buf> {
    fn attribute_value(&self, attribute: &'buf str) -> Option<&str> {
        self.attributes().get(attribute)??.try_as_utf8_str()
    }
}
