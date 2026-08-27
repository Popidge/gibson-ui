use crossbeam_channel::{Receiver, Sender, TryRecvError, bounded, unbounded};
use glam::Vec3;
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

pub const RENDER_ANCESTOR_DEPTH: usize = 1;
pub const RENDER_DESCENDANT_DEPTH: usize = 2;
pub const MAX_VISIBLE_TOWERS: usize = 128;

const MAX_INDEXED_DIRECTORIES: usize = 250_000;
const MAX_CHILDREN_PER_DIRECTORY: usize = 512;
const PRIORITY_DEPTH: usize = 2;

#[derive(Clone, Debug)]
pub struct DirectoryChild {
    pub path: PathBuf,
    pub name: String,
    pub readable: bool,
    pub traversable: bool,
}

#[derive(Clone, Debug)]
pub struct DirectoryScan {
    pub path: PathBuf,
    pub entry_count: usize,
    pub children: Vec<DirectoryChild>,
}

#[derive(Debug)]
pub enum TopologyEvent {
    Scan(DirectoryScan),
    Complete { indexed: usize, capped: bool },
}

enum TopologyCommand {
    Prioritise(PathBuf),
    Shutdown,
}

pub struct TopologyIndexer {
    command_tx: Sender<TopologyCommand>,
    pub event_rx: Receiver<TopologyEvent>,
}

impl TopologyIndexer {
    pub fn spawn(initial: PathBuf) -> anyhow::Result<Self> {
        let (command_tx, command_rx) = unbounded();
        let (event_tx, event_rx) = bounded(2_048);
        thread::Builder::new()
            .name("gibson-topology".into())
            .spawn(move || index_worker(initial, command_rx, event_tx))
            .map_err(anyhow::Error::from)?;
        Ok(Self {
            command_tx,
            event_rx,
        })
    }

    pub fn prioritise(&self, path: PathBuf) {
        let _ = self.command_tx.send(TopologyCommand::Prioritise(path));
    }
}

impl Drop for TopologyIndexer {
    fn drop(&mut self) {
        let _ = self.command_tx.send(TopologyCommand::Shutdown);
    }
}

#[derive(Clone, Debug)]
struct DirectoryNode {
    name: String,
    children: Vec<PathBuf>,
    slots: HashMap<PathBuf, usize>,
    entry_count: usize,
    readable: bool,
    scanned: bool,
}

#[derive(Clone, Debug)]
pub struct CityTowerPlacement {
    pub path: PathBuf,
    pub name: String,
    pub position: Vec3,
    pub entry_count: usize,
    pub known: bool,
    pub readable: bool,
    pub scale: f32,
}

#[derive(Default)]
pub struct DirectoryIndex {
    nodes: HashMap<PathBuf, DirectoryNode>,
}

impl DirectoryIndex {
    pub fn new(initial: &Path) -> Self {
        let mut index = Self::default();
        for path in ancestor_chain(initial) {
            index.ensure_node(path);
        }
        index
    }

    pub fn ingest(&mut self, scan: DirectoryScan) -> bool {
        self.ensure_node(scan.path.clone());
        let old_children = self
            .nodes
            .get(&scan.path)
            .map(|node| node.children.clone())
            .unwrap_or_default();

        let mut children = scan.children;
        children.sort_by(|left, right| {
            left.name
                .as_bytes()
                .cmp(right.name.as_bytes())
                .then_with(|| left.path.cmp(&right.path))
        });
        children.truncate(MAX_CHILDREN_PER_DIRECTORY);
        let child_paths = children
            .iter()
            .map(|child| child.path.clone())
            .collect::<Vec<_>>();
        for child in &children {
            self.ensure_node(child.path.clone());
            if let Some(node) = self.nodes.get_mut(&child.path) {
                node.name.clone_from(&child.name);
                node.readable = child.readable;
            }
        }

        let node = self
            .nodes
            .get_mut(&scan.path)
            .expect("the scanned directory must exist in the topology");
        let mut next_slot = node
            .slots
            .values()
            .copied()
            .max()
            .map_or(0, |slot| slot + 1);
        for child in &child_paths {
            if !node.slots.contains_key(child) {
                node.slots.insert(child.clone(), next_slot);
                next_slot += 1;
            }
        }
        let changed =
            !node.scanned || node.entry_count != scan.entry_count || old_children != child_paths;
        node.children = child_paths;
        node.entry_count = scan.entry_count;
        node.scanned = true;
        changed
    }

    pub fn visible_layout(&mut self, current: &Path) -> Vec<CityTowerPlacement> {
        for path in ancestor_chain(current) {
            self.ensure_node(path);
        }

        let mut visible = Vec::with_capacity(MAX_VISIBLE_TOWERS);
        let mut included = HashSet::new();
        let mut ancestors = current
            .ancestors()
            .skip(1)
            .take(RENDER_ANCESTOR_DEPTH)
            .map(Path::to_path_buf)
            .collect::<Vec<_>>();
        ancestors.reverse();
        for ancestor in ancestors {
            push_unique(&mut visible, &mut included, ancestor);
        }
        push_unique(&mut visible, &mut included, current.to_path_buf());

        if let Some(parent) = current.parent() {
            let siblings = self
                .nodes
                .get(parent)
                .map(|node| node.children.clone())
                .unwrap_or_default();
            for sibling in siblings.into_iter().take(MAX_CHILDREN_PER_DIRECTORY) {
                push_unique(&mut visible, &mut included, sibling);
                if visible.len() >= MAX_VISIBLE_TOWERS {
                    break;
                }
            }
        }

        let mut queue = VecDeque::from([(current.to_path_buf(), 0_usize)]);
        while let Some((path, depth)) = queue.pop_front() {
            if depth >= RENDER_DESCENDANT_DEPTH || visible.len() >= MAX_VISIBLE_TOWERS {
                continue;
            }
            let children = self
                .nodes
                .get(&path)
                .map(|node| node.children.clone())
                .unwrap_or_default();
            for child in children.into_iter().take(MAX_CHILDREN_PER_DIRECTORY) {
                push_unique(&mut visible, &mut included, child.clone());
                queue.push_back((child, depth + 1));
                if visible.len() >= MAX_VISIBLE_TOWERS {
                    break;
                }
            }
        }

        visible
            .into_iter()
            .filter_map(|path| {
                let node = self.nodes.get(&path)?;
                Some(CityTowerPlacement {
                    position: self.world_position(&path),
                    scale: tower_scale(current, &path),
                    path,
                    name: node.name.clone(),
                    entry_count: node.entry_count,
                    known: node.scanned,
                    readable: node.readable,
                })
            })
            .collect()
    }

    fn ensure_node(&mut self, path: PathBuf) {
        if self.nodes.contains_key(&path) {
            return;
        }
        let name = display_name(&path);
        self.nodes.insert(
            path,
            DirectoryNode {
                name,
                children: Vec::new(),
                slots: HashMap::new(),
                entry_count: 0,
                readable: true,
                scanned: false,
            },
        );
    }

    fn world_position(&self, path: &Path) -> Vec3 {
        if path == Path::new("/") {
            return Vec3::ZERO;
        }
        let mut position = Vec3::ZERO;
        let chain = ancestor_chain(path);
        for (depth, child) in chain.iter().enumerate().skip(1) {
            let parent = &chain[depth - 1];
            let slot = self
                .nodes
                .get(parent)
                .and_then(|node| node.slots.get(child))
                .copied()
                .unwrap_or_else(|| fallback_slot(child));
            let spacing = (8.2 * 0.90_f32.powi(depth as i32 - 1)).max(3.4);
            position += city_grid_position(slot + 1) * spacing;
        }
        position
    }
}

fn index_worker(
    initial: PathBuf,
    command_rx: Receiver<TopologyCommand>,
    event_tx: Sender<TopologyEvent>,
) {
    let mut allowed_devices = HashSet::new();
    if let Ok(metadata) = fs::symlink_metadata("/") {
        allowed_devices.insert(metadata.dev());
    }
    if let Ok(metadata) = fs::symlink_metadata(&initial) {
        allowed_devices.insert(metadata.dev());
    }
    let mut scanned = HashSet::new();
    if !scan_focus(&initial, &allowed_devices, &mut scanned, &event_tx) {
        return;
    }

    let mut queue = VecDeque::from([PathBuf::from("/")]);
    let mut capped = false;
    loop {
        loop {
            match command_rx.try_recv() {
                Ok(TopologyCommand::Prioritise(path)) => {
                    if !scan_focus(&path, &allowed_devices, &mut scanned, &event_tx) {
                        return;
                    }
                }
                Ok(TopologyCommand::Shutdown) | Err(TryRecvError::Disconnected) => return,
                Err(TryRecvError::Empty) => break,
            }
        }
        let Some(path) = queue.pop_front() else {
            break;
        };
        if scanned.len() >= MAX_INDEXED_DIRECTORIES {
            capped = true;
            break;
        }
        if scanned.contains(&path) {
            continue;
        }
        if let Some(scan) = scan_one(&path, &allowed_devices) {
            scanned.insert(path);
            queue.extend(
                scan.children
                    .iter()
                    .filter(|child| child.traversable)
                    .map(|child| child.path.clone()),
            );
            if event_tx.send(TopologyEvent::Scan(scan)).is_err() {
                return;
            }
        }
        thread::sleep(Duration::from_millis(4));
    }

    if event_tx
        .send(TopologyEvent::Complete {
            indexed: scanned.len(),
            capped,
        })
        .is_err()
    {
        return;
    }
    while let Ok(command) = command_rx.recv() {
        match command {
            TopologyCommand::Prioritise(path) => {
                if !scan_focus(&path, &allowed_devices, &mut scanned, &event_tx) {
                    return;
                }
            }
            TopologyCommand::Shutdown => return,
        }
    }
}

fn scan_focus(
    focus: &Path,
    allowed_devices: &HashSet<u64>,
    scanned: &mut HashSet<PathBuf>,
    event_tx: &Sender<TopologyEvent>,
) -> bool {
    let mut queue = VecDeque::new();
    for ancestor in ancestor_chain(focus) {
        queue.push_back((ancestor, 0_usize));
    }
    while let Some((path, depth)) = queue.pop_front() {
        if scanned.len() >= MAX_INDEXED_DIRECTORIES {
            return true;
        }
        if scanned.contains(&path) {
            continue;
        }
        let Some(scan) = scan_one(&path, allowed_devices) else {
            continue;
        };
        scanned.insert(path);
        if depth < PRIORITY_DEPTH {
            queue.extend(
                scan.children
                    .iter()
                    .filter(|child| child.traversable)
                    .map(|child| (child.path.clone(), depth + 1)),
            );
        }
        if event_tx.send(TopologyEvent::Scan(scan)).is_err() {
            return false;
        }
    }
    true
}

fn scan_one(path: &Path, allowed_devices: &HashSet<u64>) -> Option<DirectoryScan> {
    let items = fs::read_dir(path).ok()?;
    let mut entry_count = 0;
    let mut children = Vec::new();
    for item in items.take(10_000).flatten() {
        entry_count += 1;
        let Ok(file_type) = item.file_type() else {
            continue;
        };
        if !file_type.is_dir() || file_type.is_symlink() || file_type.is_socket() {
            continue;
        }
        let child_path = item.path();
        let Ok(metadata) = item.metadata() else {
            continue;
        };
        let name = item
            .file_name()
            .to_string_lossy()
            .replace(['\n', '\r', '\t'], "�");
        children.push(DirectoryChild {
            path: child_path,
            name,
            readable: metadata.mode() & 0o400 != 0,
            traversable: allowed_devices.contains(&metadata.dev()),
        });
    }
    Some(DirectoryScan {
        path: path.to_path_buf(),
        entry_count,
        children,
    })
}

fn ancestor_chain(path: &Path) -> Vec<PathBuf> {
    let mut chain = path.ancestors().map(Path::to_path_buf).collect::<Vec<_>>();
    chain.reverse();
    chain
}

fn display_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().replace(['\n', '\r', '\t'], "�"))
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "/".into())
}

fn push_unique(paths: &mut Vec<PathBuf>, seen: &mut HashSet<PathBuf>, path: PathBuf) {
    if paths.len() < MAX_VISIBLE_TOWERS && seen.insert(path.clone()) {
        paths.push(path);
    }
}

fn tower_scale(current: &Path, path: &Path) -> f32 {
    if current.starts_with(path) && current != path {
        1.14
    } else if path == current {
        1.0
    } else if !path.starts_with(current)
        || path.components().count() == current.components().count() + 1
    {
        0.82
    } else {
        0.66
    }
}

pub fn affects_visible_city(current: &Path, scanned: &Path) -> bool {
    if scanned == current {
        return true;
    }
    let current_depth = current.components().count();
    let scanned_depth = scanned.components().count();
    if scanned.starts_with(current) {
        return scanned_depth.saturating_sub(current_depth) <= RENDER_DESCENDANT_DEPTH;
    }
    if current.starts_with(scanned) {
        return current_depth.saturating_sub(scanned_depth) <= RENDER_ANCESTOR_DEPTH;
    }
    current
        .parent()
        .is_some_and(|parent| scanned.parent() == Some(parent))
}

/// Return every directory edge from `from` through the nearest common ancestor to `to`.
pub fn hierarchy_route(from: &Path, to: &Path) -> Vec<PathBuf> {
    if from == to {
        return Vec::new();
    }
    let from_chain = ancestor_chain(from);
    let to_chain = ancestor_chain(to);
    let shared = from_chain
        .iter()
        .zip(&to_chain)
        .take_while(|(left, right)| left == right)
        .count();
    let mut route = Vec::new();
    if from_chain.len() > shared {
        for index in (shared.saturating_sub(1)..from_chain.len() - 1).rev() {
            route.push(from_chain[index].clone());
        }
    }
    route.extend(to_chain.iter().skip(shared).cloned());
    route
}

fn fallback_slot(path: &Path) -> usize {
    path.file_name()
        .map(|name| {
            name.as_bytes().iter().fold(0_usize, |hash, byte| {
                hash.wrapping_mul(16_777_619) ^ usize::from(*byte)
            })
        })
        .unwrap_or_default()
        % MAX_CHILDREN_PER_DIRECTORY
}

pub fn city_grid_position(slot: usize) -> Vec3 {
    if slot == 0 {
        return Vec3::ZERO;
    }
    let n = slot as i32;
    let ring = (((n as f32 + 1.0).sqrt() - 1.0) * 0.5).ceil() as i32;
    let side = ring * 2;
    let mut maximum = (ring * 2 + 1).pow(2) - 1;
    let (x, z) = if n >= maximum - side {
        (ring - (maximum - n), -ring)
    } else {
        maximum -= side;
        if n >= maximum - side {
            (-ring, -ring + (maximum - n))
        } else {
            maximum -= side;
            if n >= maximum - side {
                (-ring + (maximum - n), ring)
            } else {
                maximum -= side;
                (ring, ring - (maximum - n))
            }
        }
    };
    Vec3::new(x as f32, 0.0, z as f32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scan(path: &str, children: &[&str]) -> DirectoryScan {
        DirectoryScan {
            path: PathBuf::from(path),
            entry_count: children.len(),
            children: children
                .iter()
                .map(|child| DirectoryChild {
                    path: PathBuf::from(path).join(child),
                    name: (*child).into(),
                    readable: true,
                    traversable: true,
                })
                .collect(),
        }
    }

    #[test]
    fn visible_city_has_one_ancestor_and_two_descendant_tiers() {
        let mut index = DirectoryIndex::new(Path::new("/one/two"));
        index.ingest(scan("/one", &["two", "sibling"]));
        index.ingest(scan("/one/two", &["three"]));
        index.ingest(scan("/one/two/three", &["four"]));
        index.ingest(scan("/one/two/three/four", &["five"]));

        let paths = index
            .visible_layout(Path::new("/one/two"))
            .into_iter()
            .map(|tower| tower.path)
            .collect::<HashSet<_>>();
        assert!(paths.contains(Path::new("/one")));
        assert!(paths.contains(Path::new("/one/sibling")));
        assert!(paths.contains(Path::new("/one/two/three/four")));
        assert!(!paths.contains(Path::new("/one/two/three/four/five")));
    }

    #[test]
    fn child_slots_do_not_move_when_a_new_name_sorts_before_them() {
        let mut index = DirectoryIndex::new(Path::new("/root"));
        index.ingest(scan("/root", &["beta"]));
        let before = index.world_position(Path::new("/root/beta"));
        index.ingest(scan("/root", &["alpha", "beta"]));
        let after = index.world_position(Path::new("/root/beta"));
        assert_eq!(before, after);
    }

    #[test]
    fn visible_city_is_hard_capped() {
        let names = (0..400).map(|index| format!("d{index}"));
        let owned = names.collect::<Vec<_>>();
        let borrowed = owned.iter().map(String::as_str).collect::<Vec<_>>();
        let current = Path::new("/root/d399");
        let mut index = DirectoryIndex::new(current);
        index.ingest(scan("/root", &borrowed));
        let visible = index.visible_layout(current);
        assert_eq!(visible.len(), MAX_VISIBLE_TOWERS);
        assert!(visible.iter().any(|tower| tower.path == current));
    }

    #[test]
    fn hierarchy_routes_visit_every_edge_through_the_common_parent() {
        assert_eq!(
            hierarchy_route(Path::new("/a/b/c"), Path::new("/a/x/y")),
            vec![
                PathBuf::from("/a/b"),
                PathBuf::from("/a"),
                PathBuf::from("/a/x"),
                PathBuf::from("/a/x/y")
            ]
        );
        let mut forward = vec![PathBuf::from("/a/b/c")];
        forward.extend(hierarchy_route(Path::new("/a/b/c"), Path::new("/a/x/y")));
        let mut reverse_with_start = vec![PathBuf::from("/a/x/y")];
        reverse_with_start.extend(hierarchy_route(Path::new("/a/x/y"), Path::new("/a/b/c")));
        reverse_with_start.reverse();
        assert_eq!(forward, reverse_with_start);
    }

    #[test]
    fn unrelated_index_work_does_not_rebuild_the_visible_city() {
        assert!(affects_visible_city(
            Path::new("/home/user/dev"),
            Path::new("/home/user/dev/project")
        ));
        assert!(affects_visible_city(
            Path::new("/home/user/dev"),
            Path::new("/home/user/music")
        ));
        assert!(!affects_visible_city(
            Path::new("/home/user/dev"),
            Path::new("/usr/share")
        ));
    }
}
