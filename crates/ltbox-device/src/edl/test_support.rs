//! Temporary firmware fixtures shared by the EDL unit tests.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Process-unique suffix for temp files/dirs. Parallel tests share a pid and
/// the macOS x86_64 runner's clock is coarse, so `{pid}-{nanos}` can collide
/// and one test's `Drop` deletes another's dir; the counter prevents that.
pub(crate) fn unique_temp_id() -> String {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    format!("{}-{nanos}-{seq}", std::process::id())
}

pub(crate) fn first_node<'a>(
    doc: &'a roxmltree::Document<'a>,
    tag: &str,
) -> roxmltree::Node<'a, 'a> {
    doc.descendants().find(|n| n.has_tag_name(tag)).unwrap()
}

pub(crate) struct TempXml(PathBuf);

impl TempXml {
    pub(crate) fn new(contents: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("ltbox-edl-wipe-plan-{}.xml", unique_temp_id()));
        std::fs::write(&path, contents).expect("write temp rawprogram");
        Self(path)
    }

    pub(crate) fn path(&self) -> PathBuf {
        self.0.clone()
    }
}

pub(crate) struct TempFirmwareDir(PathBuf);

impl TempFirmwareDir {
    pub(crate) fn new() -> Self {
        let path = std::env::temp_dir().join(format!("ltbox-edl-fw-{}", unique_temp_id()));
        std::fs::create_dir_all(&path).expect("create temp firmware dir");
        Self(path)
    }

    pub(crate) fn path(&self) -> &Path {
        &self.0
    }

    pub(crate) fn write(&self, name: &str, contents: &str) {
        std::fs::write(self.0.join(name), contents).expect("write temp firmware file");
    }

    pub(crate) fn write_bytes(&self, name: &str, contents: &[u8]) {
        std::fs::write(self.0.join(name), contents).expect("write temp firmware image");
    }
}

impl Drop for TempFirmwareDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub(crate) fn xml_names(paths: &[PathBuf]) -> Vec<String> {
    paths
        .iter()
        .filter_map(|p| p.file_name().and_then(|n| n.to_str()).map(str::to_string))
        .collect()
}

impl Drop for TempXml {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
