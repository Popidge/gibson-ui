use crossbeam_channel::{Receiver, Sender, TryRecvError, unbounded};
use inotify::{Inotify, WatchDescriptor, WatchMask};
use std::collections::HashMap;
use std::ffi::OsString;
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

const EVENT_BATCH: Duration = Duration::from_millis(33);
const HEAL_INTERVAL: Duration = Duration::from_secs(2);
const MAX_ENTRIES: usize = 10_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileKind {
    Directory,
    File,
    Executable,
    Symlink,
    Other,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileEntry {
    pub id: u128,
    pub name: OsString,
    pub display_name: String,
    pub path: PathBuf,
    pub kind: FileKind,
    pub size: u64,
    pub hidden: bool,
    pub readable: bool,
    pub modified_ns: i128,
}

impl FileEntry {
    pub fn is_directory(&self) -> bool {
        self.kind == FileKind::Directory
    }
}

#[derive(Debug)]
pub enum FsEvent {
    Snapshot {
        root: PathBuf,
        entries: Vec<FileEntry>,
        mutations: Vec<FileMutation>,
    },
    Error(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FileMutation {
    Created(FileEntry),
    Removed(FileEntry),
    Renamed { before: FileEntry, after: FileEntry },
    Modified(FileEntry),
}

enum FsCommand {
    SetRoot(PathBuf),
    Shutdown,
}

pub struct FsWatcher {
    command_tx: Sender<FsCommand>,
    pub event_rx: Receiver<FsEvent>,
}

impl FsWatcher {
    pub fn spawn(root: PathBuf) -> anyhow::Result<Self> {
        let (command_tx, command_rx) = unbounded();
        let (event_tx, event_rx) = unbounded();
        thread::Builder::new()
            .name("gibson-fs".into())
            .spawn(move || worker(root, command_rx, event_tx))
            .map_err(anyhow::Error::from)?;
        Ok(Self {
            command_tx,
            event_rx,
        })
    }

    pub fn set_root(&self, root: PathBuf) {
        let _ = self.command_tx.send(FsCommand::SetRoot(root));
    }
}

impl Drop for FsWatcher {
    fn drop(&mut self) {
        let _ = self.command_tx.send(FsCommand::Shutdown);
    }
}

fn worker(initial_root: PathBuf, command_rx: Receiver<FsCommand>, event_tx: Sender<FsEvent>) {
    let mut inotify = match Inotify::init() {
        Ok(value) => value,
        Err(error) => {
            let _ = event_tx.send(FsEvent::Error(format!("inotify init failed: {error}")));
            return;
        }
    };
    let mut buffer = vec![0_u8; 64 * 1024];
    let mut root = initial_root;
    let mut watch: Option<WatchDescriptor> = None;
    let mut previous: Option<Vec<FileEntry>> = None;
    let now = Instant::now();
    let mut dirty_since = Some(now.checked_sub(EVENT_BATCH).unwrap_or(now));
    let mut last_scan = now.checked_sub(HEAL_INTERVAL).unwrap_or(now);

    if let Err(error) = replace_watch(&inotify, &mut watch, &root) {
        let _ = event_tx.send(FsEvent::Error(error));
    }

    loop {
        loop {
            match command_rx.try_recv() {
                Ok(FsCommand::SetRoot(next_root)) => {
                    root = next_root;
                    previous = None;
                    let now = Instant::now();
                    dirty_since = Some(now.checked_sub(EVENT_BATCH).unwrap_or(now));
                    if let Err(error) = replace_watch(&inotify, &mut watch, &root) {
                        let _ = event_tx.send(FsEvent::Error(error));
                    }
                }
                Ok(FsCommand::Shutdown) | Err(TryRecvError::Disconnected) => return,
                Err(TryRecvError::Empty) => break,
            }
        }

        match inotify.read_events(&mut buffer) {
            Ok(events) => {
                for _ in events {
                    dirty_since.get_or_insert_with(Instant::now);
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(error) => {
                let _ = event_tx.send(FsEvent::Error(format!("file watch failed: {error}")));
                dirty_since.get_or_insert_with(Instant::now);
            }
        }

        let now = Instant::now();
        let batch_ready =
            dirty_since.is_some_and(|started| now.duration_since(started) >= EVENT_BATCH);
        let heal_ready = now.duration_since(last_scan) >= HEAL_INTERVAL;
        if batch_ready || heal_ready {
            match scan_directory(&root) {
                Ok(entries) => {
                    if previous.as_ref() != Some(&entries) {
                        let mutations = previous
                            .as_deref()
                            .map_or_else(Vec::new, |before| diff_entries(before, &entries));
                        previous = Some(entries.clone());
                        let _ = event_tx.send(FsEvent::Snapshot {
                            root: root.clone(),
                            entries,
                            mutations,
                        });
                    }
                }
                Err(error) => {
                    let _ = event_tx.send(FsEvent::Error(format!(
                        "cannot scan {}: {error}",
                        root.display()
                    )));
                }
            }
            dirty_since = None;
            last_scan = now;
        }

        thread::sleep(Duration::from_millis(8));
    }
}

fn diff_entries(before: &[FileEntry], after: &[FileEntry]) -> Vec<FileMutation> {
    let before_by_id = before
        .iter()
        .map(|entry| (entry.id, entry))
        .collect::<HashMap<_, _>>();
    let after_by_id = after
        .iter()
        .map(|entry| (entry.id, entry))
        .collect::<HashMap<_, _>>();
    let mut mutations = Vec::new();

    for entry in before {
        let Some(next) = after_by_id.get(&entry.id) else {
            mutations.push(FileMutation::Removed(entry.clone()));
            continue;
        };
        if entry.path != next.path {
            mutations.push(FileMutation::Renamed {
                before: entry.clone(),
                after: (*next).clone(),
            });
        } else if entry.size != next.size
            || entry.modified_ns != next.modified_ns
            || entry.kind != next.kind
            || entry.readable != next.readable
        {
            mutations.push(FileMutation::Modified((*next).clone()));
        }
    }
    for entry in after {
        if !before_by_id.contains_key(&entry.id) {
            mutations.push(FileMutation::Created(entry.clone()));
        }
    }
    mutations
}

fn replace_watch(
    inotify: &Inotify,
    current: &mut Option<WatchDescriptor>,
    root: &Path,
) -> Result<(), String> {
    if let Some(watch) = current.take() {
        let _ = inotify.watches().remove(watch);
    }
    let mask = WatchMask::CREATE
        | WatchMask::DELETE
        | WatchMask::MODIFY
        | WatchMask::ATTRIB
        | WatchMask::MOVED_FROM
        | WatchMask::MOVED_TO
        | WatchMask::DELETE_SELF
        | WatchMask::MOVE_SELF;
    let watch = inotify
        .watches()
        .add(root, mask)
        .map_err(|error| format!("cannot watch {}: {error}", root.display()))?;
    *current = Some(watch);
    Ok(())
}

pub fn scan_directory(root: &Path) -> anyhow::Result<Vec<FileEntry>> {
    let mut entries = Vec::new();
    for item in fs::read_dir(root)?.take(MAX_ENTRIES) {
        let Ok(item) = item else {
            continue;
        };
        let path = item.path();
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            continue;
        };
        let file_type = metadata.file_type();
        let mode = metadata.mode();
        let kind = if file_type.is_symlink() {
            FileKind::Symlink
        } else if file_type.is_dir() {
            FileKind::Directory
        } else if file_type.is_file() && mode & 0o111 != 0 {
            FileKind::Executable
        } else if file_type.is_file() {
            FileKind::File
        } else {
            FileKind::Other
        };
        let name = item.file_name();
        let raw_name = name.as_os_str().as_bytes();
        let hidden = raw_name.first() == Some(&b'.');
        let modified_ns =
            i128::from(metadata.mtime()) * 1_000_000_000 + i128::from(metadata.mtime_nsec());
        entries.push(FileEntry {
            id: (u128::from(metadata.dev()) << 64) | u128::from(metadata.ino()),
            display_name: name.to_string_lossy().replace(['\n', '\r', '\t'], "�"),
            name,
            path,
            kind,
            size: metadata.len(),
            hidden,
            readable: mode & 0o400 != 0,
            modified_ns,
        });
    }
    entries.sort_by(|left, right| {
        (!left.is_directory(), left.name.as_os_str().as_bytes())
            .cmp(&(!right.is_directory(), right.name.as_os_str().as_bytes()))
    });
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;
    use std::time::{Duration, Instant};

    #[test]
    fn scan_keeps_symlinks_as_symlinks() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir(temp.path().join("target")).unwrap();
        symlink(temp.path().join("target"), temp.path().join("link")).unwrap();
        let entries = scan_directory(temp.path()).unwrap();
        let link = entries
            .iter()
            .find(|entry| entry.display_name == "link")
            .unwrap();
        assert_eq!(link.kind, FileKind::Symlink);
    }

    #[test]
    fn watcher_reports_live_creates() {
        let temp = tempfile::tempdir().unwrap();
        let watcher = FsWatcher::spawn(temp.path().to_path_buf()).unwrap();
        let _ = watcher
            .event_rx
            .recv_timeout(Duration::from_secs(2))
            .unwrap();
        fs::write(temp.path().join("erupts"), b"data").unwrap();

        let deadline = Instant::now() + Duration::from_secs(3);
        let mut observed = false;
        while Instant::now() < deadline {
            if let Ok(FsEvent::Snapshot { entries, .. }) =
                watcher.event_rx.recv_timeout(Duration::from_millis(100))
                && entries.iter().any(|entry| entry.display_name == "erupts")
            {
                observed = true;
                break;
            }
        }
        assert!(observed, "the watcher did not report a new file");
    }

    #[test]
    fn snapshot_diff_distinguishes_create_remove_rename_and_modify() {
        let temp = tempfile::tempdir().unwrap();
        let original = temp.path().join("before");
        fs::write(&original, b"one").unwrap();
        let before = scan_directory(temp.path()).unwrap();

        let renamed = temp.path().join("after");
        fs::rename(&original, &renamed).unwrap();
        fs::write(&renamed, b"two two").unwrap();
        fs::write(temp.path().join("created"), b"new").unwrap();
        let after = scan_directory(temp.path()).unwrap();
        let mutations = diff_entries(&before, &after);

        assert!(mutations.iter().any(|mutation| matches!(
            mutation,
            FileMutation::Renamed { before, after }
                if before.display_name == "before" && after.display_name == "after"
        )));
        assert!(mutations.iter().any(|mutation| matches!(
            mutation,
            FileMutation::Created(entry) if entry.display_name == "created"
        )));

        let removed_before = after;
        fs::remove_file(&renamed).unwrap();
        let removed_after = scan_directory(temp.path()).unwrap();
        assert!(diff_entries(&removed_before, &removed_after)
            .iter()
            .any(|mutation| matches!(mutation, FileMutation::Removed(entry) if entry.display_name == "after")));
    }
}
