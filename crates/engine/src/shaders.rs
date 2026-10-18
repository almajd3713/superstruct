//! Shader sources. Debug builds read the files on disk so edits show up live; release builds embed them.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};

const COMMON: &str = include_str!("../shaders/common.wgsl");
const SCENE: &str = include_str!("../shaders/scene.wgsl");
const LINES: &str = include_str!("../shaders/lines.wgsl");
const CULL: &str = include_str!("../shaders/cull.wgsl");
const HIZ: &str = include_str!("../shaders/hiz.wgsl");
const SHADE: &str = include_str!("../shaders/shade.wgsl");
const HORIZON: &str = include_str!("../shaders/horizon.wgsl");
const HORIZON_CACHE: &str = include_str!("../shaders/horizon_cache.wgsl");
const HORIZON_MARCH: &str = include_str!("../shaders/horizon_march.wgsl");
const POST: &str = include_str!("../shaders/post.wgsl");

/// Fully composed WGSL for each pipeline.
#[derive(Debug, Clone)]
pub struct ShaderSources {
    pub scene: String,
    pub lines: String,
    pub cull: String,
    pub hiz: String,
    /// Generated tables, the L1 port and the cell-record compute entry.
    pub horizon_cache: String,
    pub horizon_march: String,
    pub post: String,
}

fn compose(common: &str, body: &str) -> String {
    format!("{common}\n{body}")
}

fn horizon_cache(port: &str, entry: &str) -> String {
    format!("{}\n{port}\n{entry}", crate::horizon::prelude())
}

impl ShaderSources {
    pub fn embedded() -> Self {
        Self {
            scene: compose(&compose(COMMON, SHADE), SCENE),
            lines: compose(COMMON, LINES),
            cull: CULL.to_owned(),
            hiz: HIZ.to_owned(),
            horizon_cache: horizon_cache(HORIZON, HORIZON_CACHE),
            horizon_march: compose(&compose(COMMON, SHADE), HORIZON_MARCH),
            post: POST.to_owned(),
        }
    }

    /// From disk in debug builds, falling back per file to the embedded copy; embedded in release.
    pub fn load() -> Self {
        if !cfg!(debug_assertions) {
            return Self::embedded();
        }
        let dir = shader_dir();
        let read = |name: &str, fallback: &str| {
            std::fs::read_to_string(dir.join(name)).unwrap_or_else(|_| fallback.to_owned())
        };
        let common = read("common.wgsl", COMMON);
        let shade = compose(&common, &read("shade.wgsl", SHADE));
        Self {
            scene: compose(&shade, &read("scene.wgsl", SCENE)),
            lines: compose(&common, &read("lines.wgsl", LINES)),
            cull: read("cull.wgsl", CULL),
            hiz: read("hiz.wgsl", HIZ),
            horizon_cache: horizon_cache(
                &read("horizon.wgsl", HORIZON),
                &read("horizon_cache.wgsl", HORIZON_CACHE),
            ),
            horizon_march: compose(&shade, &read("horizon_march.wgsl", HORIZON_MARCH)),
            post: read("post.wgsl", POST),
        }
    }
}

fn shader_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("shaders")
}

/// Reports edits under the shader directory. Inert in release builds.
pub struct ShaderWatcher {
    rx: Receiver<()>,
    _watcher: Option<notify::RecommendedWatcher>,
}

impl ShaderWatcher {
    pub fn new() -> Self {
        let (tx, rx) = channel();
        let watcher = cfg!(debug_assertions)
            .then(|| {
                use notify::Watcher;
                let mut w =
                    notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
                        if event.is_ok_and(|e| !e.kind.is_access()) {
                            let _ = tx.send(());
                        }
                    })
                    .ok()?;
                w.watch(&shader_dir(), notify::RecursiveMode::NonRecursive)
                    .ok()?;
                Some(w)
            })
            .flatten();
        Self {
            rx,
            _watcher: watcher,
        }
    }

    /// True if any shader changed since the last call.
    pub fn changed(&self) -> bool {
        let mut any = false;
        while self.rx.try_recv().is_ok() {
            any = true;
        }
        any
    }
}

impl Default for ShaderWatcher {
    fn default() -> Self {
        Self::new()
    }
}
