use std::borrow::Cow;
use std::fs;
use std::path::{Path, MAIN_SEPARATOR};
use std::process::exit;

use anyhow::{Context, Result};

use crate::atlas::Atlas;
use crate::index::Index;

mod atlas;
mod index;
mod point;

fn main() {
    let argv = std::env::args().collect::<Vec<String>>();
    let mut args = argv.iter().map(String::as_str).peekable();
    let exe = args
        .next()
        .map(|s| s.rsplit(MAIN_SEPARATOR).next().unwrap_or(s))
        .unwrap_or("atlast");

    #[derive(Debug)]
    enum Mode<'s> {
        Pack(&'s str),
        Unpack(&'s str),
    }

    let mut mode = Option::<Mode>::None;
    let mut output = Option::<&str>::None;
    let mut dry_run = false;
    let mut index = IndexMode::Overwrite;
    let mut size = (0, 0);

    while let Some(arg) = args.next() {
        match arg {
            "--pack" => {
                let dir = args
                    .peek()
                    .filter(|peek| !peek.starts_with('-'))
                    .map(drop)
                    .and_then(|_| args.next())
                    .unwrap_or("GameFont");
                mode.replace(Mode::Pack(dir));
            }
            "--unpack" => {
                let tga = args
                    .peek()
                    .filter(|peek| !peek.starts_with('-'))
                    .map(drop)
                    .and_then(|_| args.next())
                    .unwrap_or("GameFont.tga");
                mode.replace(Mode::Unpack(tga));
            }
            "--output" => {
                output.replace(args.next().unwrap_or_else(|| usage_and_exit(exe)));
            }
            "-n" | "--dry-run" => dry_run = true,
            "--skip-index" => index = IndexMode::Skip,
            "--patch-index" => index = IndexMode::Patch,
            "--size" => {
                size = args.next().and_then(parse_dims).unwrap_or_else(|| {
                    eprintln!("expected --size [WIDTH]x[HEIGHT]");
                    usage_and_exit(exe);
                });
            }
            "-h" | "--help" => usage_and_exit(exe),
            _ => {
                eprintln!("unexpected argument: {arg}");
                usage_and_exit(exe);
            }
        }
    }

    let Some(mode) = mode else {
        usage_and_exit(exe)
    };

    let output = output.map(Cow::from).unwrap_or_else(|| {
        /* infer output filename */
        match mode {
            Mode::Pack(dir) => Path::new(dir)
                .canonicalize()
                .ok()
                .and_then(|path| {
                    path.file_name()
                        .and_then(|osstr| osstr.to_str())
                        .map(|name| format!("{name}.tga").into())
                })
                .unwrap_or("GameFont.tga".into()),
            Mode::Unpack(tga) => Path::new(tga)
                .file_stem()
                .and_then(|osstr| osstr.to_str())
                .unwrap_or("GameFont")
                .into(),
        }
    });

    let res = match mode {
        Mode::Pack(dir) => pack_to_tga(&output, dir, dry_run, size),
        Mode::Unpack(tga) => unpack_to_dir(&output, tga, dry_run, index),
    };

    if let Err(err) = res {
        eprintln!("oopsie woopsie!");
        eprintln!("{err:#}");
    }
}

fn usage_and_exit(exe: &str) -> ! {
    eprintln!(
        r#"\
usage: {exe} [options]
  options can include:
  --unpack [GameFont.tga]    unpack GameFont.tga to a directory
  --pack [GameFont/]         opposite of unpack, write GameFont.tga using unpacked files
  -n, --dry-run              read but don't write files
  -n, --dry-run              read but don't write files
  --output ...               when used with --unpack, sets the output directory
                             when used with --pack, sets the output .tga file
  --skip-index               with --unpack, do not write index.html
  --patch-index              with --unpack, only update matching images in index.html
  --size [WIDTH]x[HEIGHT]    with --pack, sets .tga file dimensions

examples:

  {exe} --unpack
    Read `GameFont.tga` and write each glyph as a .png file in the
    `GameFont` directory with an `index.html` needed for repacking.

  {exe} --unpack GameFont_75.tga
    Unpack `GameFont_75.tga` to the `GameFont_75` directory.

  {exe} --unpack SpecialGameFont.tga --output GameFont_75 --patch-index
    Unpack `SpecialGameFont.tga` to the `GameFont_75` directory. Instead of overwriting
    `GameFont_75/index.html`, only update `<img>` elements with paths that match the image files
    unpacked from the .tga file. This could be useful if you're unpacking a .tga that contains just
    the text portion of the atlas and want to update just the descent/baseline markers for those
    images in the html file.

  {exe} --pack --output SexyLettuce.tga
    Read the `index.html` in the `GameFont` directory and pack the
    images listed there into an atlas named `SexyLettuce.tga`.

The index.html is used as a manifest for repacking GameFont.tga and contains information about
descent/baseline markers.
"#
    );
    exit(2);
}

fn pack_to_tga(destination: &str, input: &str, dry_run: bool, size: (u32, u32)) -> Result<()> {
    let ts = TimeSince::default();

    eprintln!("{ts} packing images under {input} to {destination}");
    let index_path = Path::new(input).join("index.html");
    let index_contents = fs::read_to_string(&index_path)
        .with_context(|| format!("read {}", index_path.display()))?;
    let index = Index::from_html(&index_contents)
        .with_context(|| format!("parse {}", index_path.display()))?;

    eprintln!("{ts} loading {} images...", index.len());
    let loaded_index = index.load_images(input)?;

    let atlas = loaded_index.to_atlas_image(size)?;
    eprintln!("{ts} packed {}x{}", atlas.width(), atlas.height());

    if dry_run {
        eprintln!("{ts} dry run, not writing {destination}");
        return Ok(());
    }

    atlas
        .save(destination)
        .with_context(|| format!("save {destination}"))?;
    eprintln!("{ts} written to {destination}");

    Ok(())
}

#[derive(Debug)]
enum IndexMode {
    Skip,
    Overwrite,
    Patch,
}

fn unpack_to_dir(
    destination: &str,
    input: &str,
    dry_run: bool,
    index_mode: IndexMode,
) -> Result<()> {
    let ts = TimeSince::default();

    eprintln!("{ts} loading {input} to unpack to {destination}... ");
    let img = image::open(input).with_context(|| format!("open {input}"))?;
    let buf = img.into_rgba8();
    let atlas = Atlas::from_image(&buf);
    eprintln!(
        "{ts} found {} images over {} rows",
        atlas.len(),
        atlas.row_count()
    );

    if dry_run {
        eprintln!("{ts} dry run, not saving images to {destination}");
        return Ok(());
    }

    eprintln!("{ts} saving to {destination}...");
    fs::create_dir_all(destination).context("open destination")?;

    let index = atlas
        .save_images(destination)
        .context("save atlas images")?;

    let index_path = Path::new(destination).join("index.html");

    match index_mode {
        IndexMode::Skip => (),
        IndexMode::Overwrite => {
            eprintln!("{ts} writing {}", index_path.display());
            fs::write(&index_path, index.to_html())
                .with_context(|| format!("write {}", index_path.display()))?;
        }
        IndexMode::Patch => {
            eprintln!("{ts} patching {}", index_path.display());
            let html = fs::read_to_string(&index_path)
                .with_context(|| format!("read {}", index_path.display()))?;
            let (matched, new_html) = index
                .patch_html(&html)
                .with_context(|| format!("patch {}", index_path.display()))?;
            fs::write(&index_path, new_html)
                .with_context(|| format!("write {}", index_path.display()))?;
            eprintln!("{ts} matched {matched} <img>s");
        }
    }

    eprintln!("{ts} done unpacking to {}", destination);
    Ok(())
}

fn parse_dims(s: &str) -> Option<(u32, u32)> {
    let (w, h) = s.split_once('x')?;
    Some((parse_dim(w)?, parse_dim(h)?))
}

fn parse_dim(s: &str) -> Option<u32> {
    match s {
        "" => Some(0),
        _ => s.parse().ok(),
    }
}

use std::{cell::Cell, fmt, time::Instant};

struct TimeSince(Cell<Option<Instant>>);

impl Default for TimeSince {
    fn default() -> Self {
        TimeSince(Cell::new(None))
    }
}

impl fmt::Display for TimeSince {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let now = Instant::now();
        let since = self
            .0
            .replace(Some(now))
            .map(|then| now.saturating_duration_since(then).as_secs_f32())
            .unwrap_or_default();
        write!(f, "[{since:.03}s]")
    }
}
