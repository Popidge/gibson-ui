use crate::filesystem::{FileEntry, FileKind};
use crossbeam_channel::{Receiver, Sender, TryRecvError, unbounded};
use std::collections::hash_map::DefaultHasher;
use std::fmt::Write as _;
use std::fs;
use std::hash::{Hash, Hasher};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;

const PREVIEW_CHILD_LIMIT: usize = 12;
const PREVIEW_SCAN_LIMIT: usize = 256;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NavigatorAction {
    EnterDirectory(PathBuf),
    OpenFile(PathBuf),
}

#[derive(Clone, Debug)]
pub struct NavigatorSnapshot {
    pub listing: String,
    pub visible_file_ids: Vec<Option<u128>>,
    pub listing_fingerprint: u64,
    pub metadata: String,
    pub metadata_fingerprint: u64,
    pub selected_line: usize,
    pub selection_position: f32,
}

#[derive(Clone, Debug)]
enum Preview {
    Empty,
    Loading { path: PathBuf },
    Ready { path: PathBuf, lines: Vec<String> },
    Error { path: PathBuf, message: String },
}

#[derive(Clone, Debug)]
struct PreviewRequest {
    generation: u64,
    path: PathBuf,
    directory: bool,
}

#[derive(Clone, Debug)]
struct PreviewResult {
    generation: u64,
    path: PathBuf,
    result: Result<Vec<String>, String>,
}

/// A conventional file list. It owns selection and preview state; the 3D scene does not.
pub struct Navigator {
    root: PathBuf,
    entries: Vec<FileEntry>,
    cursor: usize,
    last_window_start: usize,
    generation: u64,
    preview: Preview,
    command_tx: Sender<PreviewRequest>,
    result_rx: Receiver<PreviewResult>,
    snapshot_cache: Option<(usize, usize, bool, Arc<NavigatorSnapshot>)>,
}

impl Navigator {
    pub fn new(root: PathBuf) -> anyhow::Result<Self> {
        let (command_tx, command_rx) = unbounded();
        let (result_tx, result_rx) = unbounded();
        thread::Builder::new()
            .name("gibson-preview".into())
            .spawn(move || preview_worker(command_rx, result_tx))
            .map_err(anyhow::Error::from)?;
        let mut navigator = Self {
            root,
            entries: Vec::new(),
            cursor: 0,
            last_window_start: 0,
            generation: 0,
            preview: Preview::Empty,
            command_tx,
            result_rx,
            snapshot_cache: None,
        };
        navigator.request_preview();
        Ok(navigator)
    }

    pub fn update(&mut self, root: PathBuf, entries: Vec<FileEntry>) {
        let selected_path = self.selected_path();
        self.root = root;
        self.entries = entries;
        self.cursor = selected_path
            .and_then(|path| self.index_for_path(&path))
            .unwrap_or(0)
            .min(self.item_count().saturating_sub(1));
        self.request_preview();
    }

    pub fn poll(&mut self) {
        while let Ok(result) = self.result_rx.try_recv() {
            if result.generation != self.generation
                || self.selected_path().as_ref() != Some(&result.path)
            {
                continue;
            }
            self.snapshot_cache = None;
            self.preview = match result.result {
                Ok(lines) => Preview::Ready {
                    path: result.path,
                    lines,
                },
                Err(message) => Preview::Error {
                    path: result.path,
                    message,
                },
            };
        }
    }

    pub fn move_selection(&mut self, delta: isize) -> bool {
        let last = self.item_count().saturating_sub(1);
        let next = self.cursor.saturating_add_signed(delta).min(last);
        if next != self.cursor {
            self.cursor = next;
            self.request_preview();
            true
        } else {
            false
        }
    }

    pub fn select_first(&mut self) -> bool {
        self.select_index(0)
    }

    pub fn select_last(&mut self) -> bool {
        self.select_index(self.item_count().saturating_sub(1))
    }

    pub fn select_visible_row(&mut self, row: usize) -> bool {
        self.select_index(self.last_window_start.saturating_add(row))
    }

    fn select_index(&mut self, index: usize) -> bool {
        let index = index.min(self.item_count().saturating_sub(1));
        if index != self.cursor {
            self.cursor = index;
            self.request_preview();
            true
        } else {
            false
        }
    }

    pub fn activate(&self) -> Option<NavigatorAction> {
        let path = self.selected_path()?;
        let directory = self.cursor == 0 && self.has_parent()
            || self.selected_entry().is_some_and(FileEntry::is_directory);
        if directory {
            Some(NavigatorAction::EnterDirectory(path))
        } else {
            Some(NavigatorAction::OpenFile(path))
        }
    }

    pub fn parent_path(&self) -> Option<PathBuf> {
        self.root.parent().map(Path::to_path_buf)
    }

    pub fn selected_entry_id(&self) -> Option<u128> {
        self.selected_entry().map(|entry| entry.id)
    }

    pub fn snapshot(
        &mut self,
        rows: usize,
        columns: usize,
        focused: bool,
    ) -> Arc<NavigatorSnapshot> {
        let rows = rows.max(8);
        let columns = columns.max(24);
        if let Some((cached_rows, cached_columns, cached_focus, snapshot)) = &self.snapshot_cache
            && (*cached_rows, *cached_columns, *cached_focus) == (rows, columns, focused)
        {
            return Arc::clone(snapshot);
        }
        let list_budget = rows.saturating_sub(3).max(2);
        let count = self.item_count();
        let overflow = count > list_budget;
        let visible_rows = list_budget.saturating_sub(usize::from(overflow)).max(1);
        let half = visible_rows / 2;
        let start = self
            .cursor
            .saturating_sub(half)
            .min(count.saturating_sub(visible_rows));
        let end = (start + visible_rows).min(count);
        self.last_window_start = start;

        let mut listing = String::new();
        let control = if focused { "CONTROL" } else { "STANDBY" };
        let _ = write!(
            listing,
            "FILES // {control}\n{}\n",
            compact_path(&self.root, columns)
        );
        for index in start..end {
            let selected = index == self.cursor;
            let marker = if selected { "▶" } else { " " };
            let (kind, name) = self.item_at(index);
            let prefix = format!("{marker} {kind} ");
            let available = columns.saturating_sub(prefix.chars().count());
            listing.push_str(&prefix);
            listing.push_str(&compact(name, available));
            listing.push('\n');
        }
        if overflow {
            let _ = writeln!(listing, "  +{} ABOVE // +{} BELOW", start, count - end);
        }
        let mut metadata = String::from("METADATA //\n");
        for line in self.preview_lines().into_iter().take(8) {
            metadata.push_str(&compact(&line, columns.max(36)));
            metadata.push('\n');
        }

        let mut listing_hasher = DefaultHasher::new();
        listing.hash(&mut listing_hasher);
        let mut metadata_hasher = DefaultHasher::new();
        metadata.hash(&mut metadata_hasher);
        let visible_file_ids = (start..end)
            .map(|index| {
                index
                    .checked_sub(usize::from(self.has_parent()))
                    .and_then(|index| self.entries.get(index))
                    .map(|entry| entry.id)
            })
            .collect();
        let snapshot = Arc::new(NavigatorSnapshot {
            visible_file_ids,
            listing,
            listing_fingerprint: listing_hasher.finish(),
            metadata,
            metadata_fingerprint: metadata_hasher.finish(),
            selected_line: 2 + self.cursor.saturating_sub(start),
            selection_position: if count <= 1 {
                0.5
            } else {
                self.cursor as f32 / (count - 1) as f32
            },
        });
        self.snapshot_cache = Some((rows, columns, focused, Arc::clone(&snapshot)));
        snapshot
    }

    fn item_count(&self) -> usize {
        self.entries.len() + usize::from(self.has_parent())
    }

    fn has_parent(&self) -> bool {
        self.root.parent().is_some()
    }

    fn selected_path(&self) -> Option<PathBuf> {
        if self.cursor == 0 && self.has_parent() {
            return self.root.parent().map(Path::to_path_buf);
        }
        self.selected_entry().map(|entry| entry.path.clone())
    }

    fn selected_entry(&self) -> Option<&FileEntry> {
        let offset = usize::from(self.has_parent());
        self.cursor
            .checked_sub(offset)
            .and_then(|index| self.entries.get(index))
    }

    fn index_for_path(&self, path: &Path) -> Option<usize> {
        if self.root.parent() == Some(path) {
            return Some(0);
        }
        let offset = usize::from(self.has_parent());
        self.entries
            .iter()
            .position(|entry| entry.path == path)
            .map(|index| index + offset)
    }

    fn item_at(&self, index: usize) -> (&'static str, &str) {
        if index == 0 && self.has_parent() {
            return ("UP  ", "..");
        }
        let offset = usize::from(self.has_parent());
        let Some(entry) = self.entries.get(index.saturating_sub(offset)) else {
            return ("    ", "");
        };
        let kind = match entry.kind {
            FileKind::Directory => "DIR ",
            FileKind::Executable => "EXEC",
            FileKind::Symlink => "LINK",
            FileKind::File => "FILE",
            FileKind::Other => "DATA",
        };
        (kind, &entry.display_name)
    }

    fn request_preview(&mut self) {
        self.snapshot_cache = None;
        let Some(path) = self.selected_path() else {
            self.preview = Preview::Empty;
            return;
        };
        self.generation = self.generation.wrapping_add(1);
        let directory = self.cursor == 0 && self.has_parent()
            || self.selected_entry().is_some_and(FileEntry::is_directory);
        self.preview = Preview::Loading { path: path.clone() };
        let _ = self.command_tx.send(PreviewRequest {
            generation: self.generation,
            path,
            directory,
        });
    }

    fn preview_lines(&self) -> Vec<String> {
        match &self.preview {
            Preview::Empty => vec!["EMPTY DIRECTORY".into()],
            Preview::Loading { path } => {
                vec![compact_path(path, 80), "READING FILE-SYSTEM DATA...".into()]
            }
            Preview::Ready { path, lines } => {
                let mut output = vec![compact_path(path, 80)];
                output.extend(lines.iter().cloned());
                output
            }
            Preview::Error { path, message } => {
                vec![compact_path(path, 80), format!("UNAVAILABLE // {message}")]
            }
        }
    }
}

fn preview_worker(command_rx: Receiver<PreviewRequest>, result_tx: Sender<PreviewResult>) {
    loop {
        let Ok(mut request) = command_rx.recv() else {
            return;
        };
        loop {
            match command_rx.try_recv() {
                Ok(next) => request = next,
                Err(TryRecvError::Disconnected) => return,
                Err(TryRecvError::Empty) => break,
            }
        }
        let result = inspect_path(&request.path, request.directory);
        let _ = result_tx.send(PreviewResult {
            generation: request.generation,
            path: request.path,
            result,
        });
    }
}

fn inspect_path(path: &Path, directory: bool) -> Result<Vec<String>, String> {
    let metadata = fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    let mode = metadata.mode() & 0o7777;
    let mut lines = vec![format!(
        "MODE {:04o} // UID {} // GID {}",
        mode,
        metadata.uid(),
        metadata.gid()
    )];
    if directory {
        let mut names = Vec::new();
        let mut count = 0_usize;
        let entries = fs::read_dir(path).map_err(|error| error.to_string())?;
        for entry in entries.take(PREVIEW_SCAN_LIMIT + 1).flatten() {
            count += 1;
            if names.len() < PREVIEW_CHILD_LIMIT {
                names.push(entry.file_name().to_string_lossy().into_owned());
            }
        }
        names.sort();
        let count_text = if count > PREVIEW_SCAN_LIMIT {
            format!("{PREVIEW_SCAN_LIMIT}+")
        } else {
            count.to_string()
        };
        lines.push(format!("DIRECTORY // {count_text} ITEMS"));
        lines.extend(names.into_iter().map(|name| format!("  {name}")));
    } else {
        lines.push(format!("SIZE {} BYTES", metadata.len()));
        lines.push(format!(
            "INODE {} // BLOCKS {}",
            metadata.ino(),
            metadata.blocks()
        ));
        lines.push(format!("MODIFIED {}", metadata.mtime()));
    }
    Ok(lines)
}

fn compact(value: &str, limit: usize) -> String {
    if value.chars().count() <= limit {
        return value.to_owned();
    }
    if limit <= 1 {
        return "…".into();
    }
    let mut result = value.chars().take(limit - 1).collect::<String>();
    result.push('…');
    result
}

fn compact_path(path: &Path, limit: usize) -> String {
    compact(&path.to_string_lossy(), limit)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    fn entry(root: &Path, name: &str, kind: FileKind, id: u128) -> FileEntry {
        FileEntry {
            id,
            name: OsString::from(name),
            display_name: name.into(),
            path: root.join(name),
            kind,
            size: 0,
            readable: true,
            modified_ns: 0,
        }
    }

    #[test]
    fn parent_is_the_first_conventional_list_item() {
        let root = PathBuf::from("/tmp/gibson-test");
        let mut navigator = Navigator::new(root.clone()).unwrap();
        navigator.update(
            root.clone(),
            vec![entry(&root, "docs", FileKind::Directory, 1)],
        );
        assert_eq!(
            navigator.activate(),
            Some(NavigatorAction::EnterDirectory(PathBuf::from("/tmp")))
        );
        navigator.move_selection(1);
        assert_eq!(
            navigator.activate(),
            Some(NavigatorAction::EnterDirectory(root.join("docs")))
        );
    }

    #[test]
    fn snapshot_cache_tracks_selection_preview_focus_size_and_directory() {
        let root = PathBuf::from("/tmp/gibson-cache-test");
        let mut navigator = Navigator::new(root.clone()).unwrap();
        let (results, result_rx) = unbounded();
        navigator.result_rx = result_rx;
        navigator.update(root.clone(), vec![entry(&root, "a", FileKind::File, 1)]);
        let first = navigator.snapshot(12, 50, true);
        assert!(Arc::ptr_eq(&first, &navigator.snapshot(12, 50, true)));
        navigator.move_selection(1);
        let selected = navigator.snapshot(12, 50, true);
        assert_ne!(first.listing_fingerprint, selected.listing_fingerprint);
        results
            .send(PreviewResult {
                generation: navigator.generation,
                path: root.join("a"),
                result: Ok(vec!["SIZE 123 BYTES".into()]),
            })
            .unwrap();
        navigator.poll();
        let preview = navigator.snapshot(12, 50, true);
        assert!(preview.metadata.contains("SIZE 123 BYTES"));
        assert_eq!(preview.listing_fingerprint, selected.listing_fingerprint);
        assert!(
            !navigator
                .snapshot(12, 50, false)
                .listing
                .contains("CONTROL")
        );
        assert!(!Arc::ptr_eq(&preview, &navigator.snapshot(20, 80, true)));
        navigator.update(root.join("empty"), Vec::new());
        assert!(!navigator.snapshot(12, 50, true).listing.contains("FILE a"));
    }

    #[test]
    fn snapshots_mark_the_selected_row() {
        let root = PathBuf::from("/tmp/gibson-test");
        let mut navigator = Navigator::new(root.clone()).unwrap();
        navigator.update(root.clone(), vec![entry(&root, "a", FileKind::File, 1)]);
        navigator.move_selection(1);
        let snapshot = navigator.snapshot(12, 50, true);
        assert!(snapshot.listing.contains("▶ FILE a"));
        assert_eq!(snapshot.selected_line, 3);
    }
}
