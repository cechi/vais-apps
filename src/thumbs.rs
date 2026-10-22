//! Thumbnails for the thumbnail view. Images are decoded and scaled on the
//! background executor, a few at a time, and the result is kept as a small
//! PNG in the user's cache directory (keyed by path, size and mtime) so the
//! next visit shows it at once. Panels ask with [`request`] while rendering
//! and are re-rendered when a thumbnail they asked for is done.

use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, VecDeque};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui_kit::*;

use crate::fs::Entry;
use crate::panel::FilePanel;

/// Side of the generated thumbnails, in pixels.
const SIZE: u32 = 256;
/// How many thumbnails are generated at the same time.
const MAX_IN_FLIGHT: usize = 4;
/// File extensions (lowercase) thumbnails are made for.
const EXTENSIONS: [&str; 11] = [
    "png", "jpg", "jpeg", "gif", "webp", "bmp", "tiff", "tif", "ico", "qoi", "svg",
];

/// Whether a thumbnail can be made for this file name.
pub fn supports(name: &str) -> bool {
    extension_of(name).is_some_and(|ext| EXTENSIONS.contains(&ext.as_str()))
}

fn extension_of(name: &str) -> Option<String> {
    let dot = name.rfind('.')?;
    (dot > 0 && dot + 1 < name.len()).then(|| name[dot + 1..].to_lowercase())
}

#[derive(Clone)]
enum State {
    Pending,
    Ready(Arc<Path>),
    Failed,
}

struct Job {
    source: PathBuf,
    target: PathBuf,
}

pub struct Thumbnailer {
    dir: Option<PathBuf>,
    states: HashMap<PathBuf, State>,
    queue: VecDeque<Job>,
    in_flight: usize,
    /// Panels to re-render when a thumbnail finishes.
    waiters: Vec<WeakEntity<FilePanel>>,
}

struct Handle(Entity<Thumbnailer>);

impl Global for Handle {}

pub fn init(cx: &mut App) {
    let dir = directories::ProjectDirs::from("", "", "vais-commander")
        .map(|d| d.cache_dir().join("thumbs"));
    let thumbnailer = cx.new(|_| Thumbnailer {
        dir,
        states: HashMap::new(),
        queue: VecDeque::new(),
        in_flight: 0,
        waiters: Vec::new(),
    });
    cx.set_global(Handle(thumbnailer));
}

/// The thumbnail of `entry` when it is ready. Otherwise generation is
/// started (once) and `panel` is re-rendered when it is done.
pub fn request(cx: &mut App, entry: &Entry, panel: WeakEntity<FilePanel>) -> Option<Arc<Path>> {
    if entry.is_dir() || entry.is_parent || !supports(&entry.name) {
        return None;
    }
    let handle = cx.global::<Handle>().0.clone();
    handle.update(cx, |this, cx| this.request(entry, panel, cx))
}

impl Thumbnailer {
    fn request(
        &mut self,
        entry: &Entry,
        panel: WeakEntity<FilePanel>,
        cx: &mut Context<Self>,
    ) -> Option<Arc<Path>> {
        let source = entry.local_path()?.to_path_buf();
        match self.states.get(&source) {
            Some(State::Ready(path)) => return Some(path.clone()),
            Some(State::Failed) => return None,
            Some(State::Pending) => {
                self.add_waiter(panel);
                return None;
            }
            None => {}
        }
        let dir = self.dir.clone()?;
        let target = dir.join(format!("{:016x}.png", key_of(entry)));
        if target.is_file() {
            let path: Arc<Path> = Arc::from(target);
            self.states.insert(source, State::Ready(path.clone()));
            return Some(path);
        }
        self.states.insert(source.clone(), State::Pending);
        self.queue.push_back(Job { source, target });
        self.add_waiter(panel);
        self.pump(cx);
        None
    }

    fn add_waiter(&mut self, panel: WeakEntity<FilePanel>) {
        if !self
            .waiters
            .iter()
            .any(|w| w.entity_id() == panel.entity_id())
        {
            self.waiters.push(panel);
        }
    }

    /// Start queued jobs while there is room.
    fn pump(&mut self, cx: &mut Context<Self>) {
        while self.in_flight < MAX_IN_FLIGHT {
            let Some(job) = self.queue.pop_front() else {
                break;
            };
            self.in_flight += 1;
            let source = job.source.clone();
            let task = cx.background_spawn(async move { generate(&job.source, &job.target) });
            cx.spawn(async move |this, cx| {
                let result = task.await;
                this.update(cx, |this, cx| {
                    this.in_flight -= 1;
                    let state = match result {
                        Ok(target) => State::Ready(Arc::from(target)),
                        Err(err) => {
                            log::debug!("thumbnail for {}: {err}", source.display());
                            State::Failed
                        }
                    };
                    this.states.insert(source, state);
                    this.pump(cx);
                    // The panels will ask again; their next render finds the file.
                    for panel in std::mem::take(&mut this.waiters) {
                        panel.update(cx, |_, cx| cx.notify()).ok();
                    }
                })
                .ok();
            })
            .detach();
        }
    }
}

/// Cache key: the path with the file's size and modification time, so an
/// edited image gets a fresh thumbnail.
fn key_of(entry: &Entry) -> u64 {
    let mut hasher = DefaultHasher::new();
    entry.location.display().hash(&mut hasher);
    entry.size.hash(&mut hasher);
    entry
        .modified
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos())
        .hash(&mut hasher);
    hasher.finish()
}

/// Decode `source`, scale it to fit [`SIZE`] and save it as `target`.
fn generate(source: &Path, target: &Path) -> anyhow::Result<PathBuf> {
    let is_svg = source
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("svg"));
    let image = if is_svg {
        render_svg(source)?
    } else {
        image::ImageReader::open(source)?
            .with_guessed_format()?
            .decode()?
            .thumbnail(SIZE, SIZE)
            .to_rgba8()
    };
    if let Some(dir) = target.parent() {
        std::fs::create_dir_all(dir)?;
    }
    // Write next to the target first so a reader never sees a half file.
    let partial = target.with_extension("part");
    image.save_with_format(&partial, image::ImageFormat::Png)?;
    std::fs::rename(&partial, target)?;
    Ok(target.to_path_buf())
}

/// Rasterise an SVG so that its longer side is [`SIZE`].
fn render_svg(source: &Path) -> anyhow::Result<image::RgbaImage> {
    let data = std::fs::read(source)?;
    let tree = usvg::Tree::from_data(&data, &usvg::Options::default())?;
    let size = tree.size();
    let scale = SIZE as f32 / size.width().max(size.height());
    let width = (size.width() * scale).ceil().max(1.0) as u32;
    let height = (size.height() * scale).ceil().max(1.0) as u32;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(width, height)
        .ok_or_else(|| anyhow::anyhow!("svg too large"))?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    // tiny-skia stores premultiplied RGBA; PNGs want straight alpha.
    let mut rgba = pixmap.data().to_vec();
    for px in rgba.chunks_exact_mut(4) {
        let a = px[3] as u32;
        if a > 0 && a < 255 {
            for c in &mut px[..3] {
                *c = ((*c as u32 * 255 + a / 2) / a).min(255) as u8;
            }
        }
    }
    image::RgbaImage::from_raw(width, height, rgba)
        .ok_or_else(|| anyhow::anyhow!("svg raster size mismatch"))
}

#[cfg(test)]
mod tests {
    use super::supports;

    #[test]
    fn supported_extensions_are_case_insensitive() {
        assert!(supports("photo.JPG"));
        assert!(supports("logo.svg"));
        assert!(supports("a.b.webp"));
        assert!(!supports("notes.txt"));
        assert!(!supports(".png"));
        assert!(!supports("png"));
    }
}
