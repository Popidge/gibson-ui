use crossbeam_channel::{Receiver, Sender, unbounded};
use percent_encoding::percent_decode;
use portable_pty::{CommandBuilder, MasterPty, NativePtySystem, PtySize, PtySystem};
use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::hash::{Hash, Hasher};
use std::io::{Read, Write};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

const INITIAL_ROWS: u16 = 24;
const INITIAL_COLS: u16 = 100;
const SCROLLBACK_LINES: usize = 10_000;
const NAVIGATION_SEQUENCE: &[u8] = b"\x1b[99~";

#[derive(Clone, Debug)]
pub enum TerminalEvent {
    Cwd(PathBuf),
    NavigationFailed,
    Prompt,
    Exited,
    Error(String),
}

enum TerminalCommand {
    Input(Vec<u8>),
    Resize(PtySize),
    Navigate(PathBuf),
    Shutdown,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum TerminalColor {
    Default,
    Indexed(u8),
    Rgb(u8, u8, u8),
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct TerminalSpan {
    pub text: String,
    pub foreground: TerminalColor,
    pub background: TerminalColor,
    pub bold: bool,
    pub dim: bool,
    pub italic: bool,
    pub underline: bool,
    pub inverse: bool,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct TerminalBackground {
    pub row: u16,
    pub column: u16,
    pub cells: u16,
    pub color: TerminalColor,
}

#[derive(Debug)]
pub struct TerminalSnapshot {
    pub spans: Vec<TerminalSpan>,
    pub backgrounds: Vec<TerminalBackground>,
    pub fingerprint: u64,
    pub cursor: (u16, u16),
    pub rows: u16,
    pub columns: u16,
}

#[derive(Clone)]
struct ParserCallbacks {
    event_tx: Sender<TerminalEvent>,
}

impl vt100::Callbacks for ParserCallbacks {
    fn unhandled_osc(&mut self, _screen: &mut vt100::Screen, params: &[&[u8]]) {
        match params.first().copied() {
            Some(b"7") => {
                let mut payload = Vec::new();
                for (index, part) in params.iter().skip(1).enumerate() {
                    if index > 0 {
                        payload.push(b';');
                    }
                    payload.extend_from_slice(part);
                }
                if let Some(path) = osc7_path(&payload) {
                    let _ = self.event_tx.send(TerminalEvent::Cwd(path));
                }
            }
            Some(b"777") => match params.get(1).copied() {
                Some(b"prompt") => {
                    let _ = self.event_tx.send(TerminalEvent::Prompt);
                }
                Some(b"navigation-failed") => {
                    let _ = self.event_tx.send(TerminalEvent::NavigationFailed);
                }
                _ => {}
            },
            _ => {}
        }
    }
}

type Parser = vt100::Parser<ParserCallbacks>;

pub struct Terminal {
    command_tx: Sender<TerminalCommand>,
    parser: Arc<Mutex<Parser>>,
    revision: Arc<AtomicU64>,
    snapshot_cache: Mutex<Option<(u64, Arc<TerminalSnapshot>)>>,
    pub event_rx: Receiver<TerminalEvent>,
}

impl Terminal {
    pub fn spawn(root: &Path) -> anyhow::Result<Self> {
        Self::spawn_with_shell(root, None)
    }

    fn spawn_with_shell(root: &Path, shell: Option<OsString>) -> anyhow::Result<Self> {
        let pty_system = NativePtySystem::default();
        let pair = pty_system.openpty(PtySize {
            rows: INITIAL_ROWS,
            cols: INITIAL_COLS,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        let (command_tx, command_rx) = unbounded();
        let (event_tx, event_rx) = unbounded();
        let parser = Arc::new(Mutex::new(Parser::new_with_callbacks(
            INITIAL_ROWS,
            INITIAL_COLS,
            SCROLLBACK_LINES,
            ParserCallbacks {
                event_tx: event_tx.clone(),
            },
        )));
        let revision = Arc::new(AtomicU64::new(0));

        let mut reader = pair.master.try_clone_reader()?;
        let reader_parser = parser.clone();
        let reader_revision = revision.clone();
        let reader_events = event_tx.clone();
        thread::Builder::new()
            .name("gibson-pty-reader".into())
            .spawn(move || {
                let mut buffer = [0_u8; 16 * 1024];
                loop {
                    match reader.read(&mut buffer) {
                        Ok(0) => break,
                        Ok(count) => {
                            let mut parser = reader_parser
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner);
                            reader_revision.fetch_add(1, Ordering::AcqRel);
                            parser.process(&buffer[..count]);
                            reader_revision.fetch_add(1, Ordering::Release);
                            drop(parser);
                        }
                        Err(error) => {
                            report_terminal_failure(
                                &reader_events,
                                format!("PTY read failed: {error}"),
                            );
                            break;
                        }
                    }
                }
            })?;

        let root = root.to_path_buf();
        thread::Builder::new()
            .name("gibson-pty-control".into())
            .spawn(move || manager(pair.master, pair.slave, root, shell, command_rx, event_tx))?;

        Ok(Self {
            command_tx,
            parser,
            revision,
            snapshot_cache: Mutex::new(None),
            event_rx,
        })
    }

    pub fn send_input(&self, bytes: impl Into<Vec<u8>>) {
        let _ = self.command_tx.send(TerminalCommand::Input(bytes.into()));
    }

    pub fn navigate(&self, path: PathBuf) {
        let _ = self.command_tx.send(TerminalCommand::Navigate(path));
    }

    pub fn resize(&self, rows: u16, cols: u16, pixel_width: u16, pixel_height: u16) {
        let rows = rows.max(2);
        let cols = cols.max(10);
        let mut parser = self
            .parser
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.revision.fetch_add(1, Ordering::AcqRel);
        parser.screen_mut().set_size(rows, cols);
        self.revision.fetch_add(1, Ordering::Release);
        drop(parser);
        let _ = self.command_tx.send(TerminalCommand::Resize(PtySize {
            rows,
            cols,
            pixel_width,
            pixel_height,
        }));
    }

    pub fn snapshot(&self) -> Arc<TerminalSnapshot> {
        let revision = self.revision.load(Ordering::Acquire);
        if revision.is_multiple_of(2) {
            let cache = self
                .snapshot_cache
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some((cached_revision, snapshot)) = cache.as_ref()
                && *cached_revision == revision
                && self.revision.load(Ordering::Acquire) == revision
            {
                return snapshot.clone();
            }
        }
        let parser = self
            .parser
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let screen = parser.screen();
        let (rows, cols) = screen.size();
        let spans = styled_spans(screen, rows, cols);
        let backgrounds = terminal_backgrounds(screen, rows, cols);
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        spans.hash(&mut hasher);
        backgrounds.hash(&mut hasher);
        let snapshot = Arc::new(TerminalSnapshot {
            spans,
            backgrounds,
            fingerprint: hasher.finish(),
            cursor: screen.cursor_position(),
            rows,
            columns: cols,
        });
        // Read the revision while the parser is locked: a later revision may describe
        // output that is absent from this snapshot.
        let revision = self.revision.load(Ordering::Acquire);
        drop(parser);
        if revision.is_multiple_of(2) {
            let mut cache = self
                .snapshot_cache
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            *cache = Some((revision, snapshot.clone()));
        }
        snapshot
    }
}

fn terminal_backgrounds(screen: &vt100::Screen, rows: u16, cols: u16) -> Vec<TerminalBackground> {
    let mut backgrounds = Vec::new();
    for row in 0..rows {
        let mut run: Option<TerminalBackground> = None;
        for column in 0..cols {
            let color = screen
                .cell(row, column)
                .map_or(TerminalColor::Default, |cell| {
                    if cell.inverse() {
                        terminal_color(cell.fgcolor())
                    } else {
                        terminal_color(cell.bgcolor())
                    }
                });
            if color == TerminalColor::Default {
                if let Some(run) = run.take() {
                    backgrounds.push(run);
                }
            } else if let Some(active) = &mut run {
                if active.color == color && active.column + active.cells == column {
                    active.cells += 1;
                } else {
                    backgrounds.push(*active);
                    *active = TerminalBackground {
                        row,
                        column,
                        cells: 1,
                        color,
                    };
                }
            } else {
                run = Some(TerminalBackground {
                    row,
                    column,
                    cells: 1,
                    color,
                });
            }
        }
        if let Some(run) = run {
            backgrounds.push(run);
        }
    }
    backgrounds
}

fn styled_spans(screen: &vt100::Screen, rows: u16, cols: u16) -> Vec<TerminalSpan> {
    let mut spans: Vec<TerminalSpan> = Vec::new();
    for row in 0..rows {
        for col in 0..cols {
            let Some(cell) = screen.cell(row, col) else {
                continue;
            };
            if cell.is_wide_continuation() {
                continue;
            }
            let text = if cell.has_contents() {
                cell.contents()
            } else {
                " "
            };
            let foreground = terminal_color(cell.fgcolor());
            let background = terminal_color(cell.bgcolor());
            let bold = cell.bold();
            let dim = cell.dim();
            let italic = cell.italic();
            let underline = cell.underline();
            let inverse = cell.inverse();
            if let Some(previous) = spans.last_mut()
                && previous.foreground == foreground
                && previous.background == background
                && previous.bold == bold
                && previous.dim == dim
                && previous.italic == italic
                && previous.underline == underline
                && previous.inverse == inverse
            {
                previous.text.push_str(text);
            } else {
                spans.push(TerminalSpan {
                    text: text.into(),
                    foreground,
                    background,
                    bold,
                    dim,
                    italic,
                    underline,
                    inverse,
                });
            }
        }
        if row + 1 < rows
            && let Some(previous) = spans.last_mut()
        {
            previous.text.push('\n');
        }
    }
    spans
}

fn terminal_color(color: vt100::Color) -> TerminalColor {
    match color {
        vt100::Color::Default => TerminalColor::Default,
        vt100::Color::Idx(index) => TerminalColor::Indexed(index),
        vt100::Color::Rgb(red, green, blue) => TerminalColor::Rgb(red, green, blue),
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        let _ = self.command_tx.send(TerminalCommand::Shutdown);
    }
}

fn manager(
    master: Box<dyn MasterPty + Send>,
    slave: Box<dyn portable_pty::SlavePty + Send>,
    root: PathBuf,
    shell: Option<OsString>,
    command_rx: Receiver<TerminalCommand>,
    event_tx: Sender<TerminalEvent>,
) {
    let runtime = match tempfile::Builder::new().prefix("gibson-").tempdir() {
        Ok(value) => value,
        Err(error) => {
            report_terminal_failure(&event_tx, format!("runtime directory failed: {error}"));
            return;
        }
    };
    let rc_path = runtime.path().join("bashrc");
    let control_path = runtime.path().join("control");
    if let Err(error) = write_runtime_files(&rc_path, &control_path) {
        report_terminal_failure(&event_tx, format!("shell bridge failed: {error}"));
        return;
    }

    let shell = shell
        .or_else(|| std::env::var_os("SHELL"))
        .unwrap_or_else(|| OsString::from("/bin/bash"));
    let is_bash = Path::new(&shell)
        .file_name()
        .is_some_and(|name| name.as_bytes().ends_with(b"bash"));
    let mut command = CommandBuilder::new(&shell);
    if is_bash {
        command.args(["--noprofile", "--rcfile"]);
        command.arg(&rc_path);
    }
    command.arg("-i");
    command.cwd(&root);
    command.env("TERM", "xterm-256color");
    command.env("COLORTERM", "truecolor");
    command.env("GIBSON", "1");
    command.env("GIBSON_CONTROL_FILE", &control_path);

    let mut child = match slave.spawn_command(command) {
        Ok(value) => value,
        Err(error) => {
            report_terminal_failure(&event_tx, format!("shell start failed: {error}"));
            return;
        }
    };
    drop(slave);
    let mut writer = match master.take_writer() {
        Ok(value) => value,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            report_terminal_failure(&event_tx, format!("PTY writer failed: {error}"));
            return;
        }
    };

    loop {
        match command_rx.recv_timeout(Duration::from_millis(16)) {
            Ok(TerminalCommand::Input(bytes)) => {
                if let Err(error) = writer.write_all(&bytes).and_then(|()| writer.flush()) {
                    let _ =
                        event_tx.send(TerminalEvent::Error(format!("PTY input failed: {error}")));
                    break;
                }
            }
            Ok(TerminalCommand::Resize(size)) => {
                if let Err(error) = master.resize(size) {
                    let _ =
                        event_tx.send(TerminalEvent::Error(format!("PTY resize failed: {error}")));
                }
            }
            Ok(TerminalCommand::Navigate(path)) => {
                if let Err(error) = write_control_path(&control_path, &path)
                    .and_then(|()| writer.write_all(NAVIGATION_SEQUENCE))
                    .and_then(|()| writer.flush())
                {
                    let _ = event_tx.send(TerminalEvent::NavigationFailed);
                    let _ = event_tx.send(TerminalEvent::Error(format!(
                        "shell navigation failed: {error}"
                    )));
                }
            }
            Ok(TerminalCommand::Shutdown)
            | Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                break;
            }
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
        }

        match child.try_wait() {
            Ok(Some(_)) => {
                break;
            }
            Ok(None) => {}
            Err(error) => {
                let _ = event_tx.send(TerminalEvent::Error(format!("shell wait failed: {error}")));
                break;
            }
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    let _ = event_tx.send(TerminalEvent::Exited);
}

fn report_terminal_failure(event_tx: &Sender<TerminalEvent>, message: String) {
    let _ = event_tx.send(TerminalEvent::Error(message));
    let _ = event_tx.send(TerminalEvent::Exited);
}

fn write_runtime_files(rc_path: &Path, control_path: &Path) -> std::io::Result<()> {
    OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .mode(0o600)
        .open(control_path)?;
    fs::write(rc_path, BASH_RC)
}

fn write_control_path(control_path: &Path, target: &Path) -> std::io::Result<()> {
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .mode(0o600)
        .open(control_path)?;
    file.write_all(target.as_os_str().as_bytes())?;
    file.write_all(&[0])?;
    file.flush()
}

fn osc7_path(payload: &[u8]) -> Option<PathBuf> {
    let rest = payload.strip_prefix(b"file://")?;
    let path_start = rest.iter().position(|byte| *byte == b'/')?;
    let decoded = percent_decode(&rest[path_start..]).collect::<Vec<_>>();
    if decoded.first() != Some(&b'/') {
        return None;
    }
    Some(PathBuf::from(OsString::from_vec(decoded)))
}

const BASH_RC: &[u8] = br#"
if [[ -f "$HOME/.bashrc" ]]; then
  source "$HOME/.bashrc"
fi

__gibson_emit_cwd() {
  printf '\033]7;file://gibson%s\007' "$PWD"
}

__gibson_prompt_dispatch() {
  __gibson_emit_cwd
  printf '\033]777;prompt\007'
}

__gibson_prepare_cd_from_ui() {
  __gibson_saved_readline_line="$READLINE_LINE"
  __gibson_saved_readline_point="$READLINE_POINT"
  local __gibson_target=""
  IFS= read -r -d '' __gibson_target < "$GIBSON_CONTROL_FILE" || true
  if [[ -z "$__gibson_target" ]] || ! builtin cd -- "$__gibson_target"; then
    printf '\033]777;navigation-failed\007'
  fi
  READLINE_LINE=""
  READLINE_POINT=0
}

__gibson_restore_readline_line() {
  READLINE_LINE="${__gibson_saved_readline_line:-}"
  READLINE_POINT="${__gibson_saved_readline_point:-0}"
  unset __gibson_saved_readline_line __gibson_saved_readline_point
}

if [[ "$(declare -p PROMPT_COMMAND 2>/dev/null)" == "declare -a"* ]]; then
  PROMPT_COMMAND+=(__gibson_prompt_dispatch)
elif [[ -n "${PROMPT_COMMAND:-}" ]]; then
  PROMPT_COMMAND=("$PROMPT_COMMAND" __gibson_prompt_dispatch)
else
  PROMPT_COMMAND=(__gibson_prompt_dispatch)
fi
bind -x '"\e[98~":__gibson_prepare_cd_from_ui'
bind -x '"\e[97~":__gibson_restore_readline_line'
bind '"\e[99~":"\e[98~\C-m\e[97~"'
export TERM=xterm-256color
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::time::{Duration, Instant};

    #[test]
    fn parses_osc7_paths() {
        let path = osc7_path(b"file://host/tmp/hello%20world").unwrap();
        assert_eq!(path, PathBuf::from("/tmp/hello world"));
    }

    #[test]
    fn rejects_non_file_osc_values() {
        assert!(osc7_path(b"https://example.test/path").is_none());
    }

    #[test]
    fn parses_navigation_failure_marker() {
        let (event_tx, event_rx) = unbounded();
        let mut parser = Parser::new_with_callbacks(2, 20, 0, ParserCallbacks { event_tx });
        parser.process(b"\x1b]777;navigation-failed\x07");
        assert!(matches!(
            event_rx.recv_timeout(Duration::from_secs(1)),
            Ok(TerminalEvent::NavigationFailed)
        ));
    }

    #[test]
    fn snapshot_preserves_ansi_cell_style() {
        let (event_tx, _event_rx) = unbounded();
        let mut parser = Parser::new_with_callbacks(2, 20, 0, ParserCallbacks { event_tx });
        parser.process(b"\x1b[1;31;44mDANGER\x1b[0m");
        let spans = styled_spans(parser.screen(), 2, 20);
        let danger = spans
            .iter()
            .find(|span| span.text.contains("DANGER"))
            .unwrap();
        assert_eq!(danger.foreground, TerminalColor::Indexed(1));
        assert!(danger.bold);
        assert_eq!(
            terminal_backgrounds(parser.screen(), 2, 20),
            vec![TerminalBackground {
                row: 0,
                column: 0,
                cells: 6,
                color: TerminalColor::Indexed(4),
            }]
        );
    }

    #[test]
    fn pty_runs_bash_and_accepts_scene_navigation() {
        let temp = tempfile::tempdir().unwrap();
        let child = temp.path().join("child directory");
        fs::create_dir(&child).unwrap();
        let terminal =
            Terminal::spawn_with_shell(temp.path(), Some(OsString::from("/bin/bash"))).unwrap();
        wait_for_prompt(&terminal, Duration::from_secs(5));

        terminal.navigate(child.clone());
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut navigated = false;
        while Instant::now() < deadline {
            if let Ok(TerminalEvent::Cwd(path)) =
                terminal.event_rx.recv_timeout(Duration::from_millis(100))
                && path == child
            {
                navigated = true;
                break;
            }
        }
        assert!(navigated, "Bash did not report the requested directory");

        thread::sleep(Duration::from_millis(50));
        let before_output = terminal.snapshot();
        let cached = terminal.snapshot();
        assert!(Arc::ptr_eq(&before_output, &cached));
        terminal.send_input(b"printf 'GIBSON_PTY_CWD=%s\\n' \"$PWD\"\r".to_vec());
        wait_for_prompt(&terminal, Duration::from_secs(5));
        thread::sleep(Duration::from_millis(50));
        let snapshot = terminal.snapshot();
        assert!(!Arc::ptr_eq(&before_output, &snapshot));
        let expected = format!("GIBSON_PTY_CWD={}", child.display());
        assert!(
            snapshot
                .spans
                .iter()
                .any(|span| span.text.contains(&expected)),
            "the live shell did not change to {}",
            child.display()
        );
    }

    #[test]
    fn pty_reports_rejected_scene_navigation() {
        let temp = tempfile::tempdir().unwrap();
        let terminal =
            Terminal::spawn_with_shell(temp.path(), Some(OsString::from("/bin/bash"))).unwrap();
        wait_for_prompt(&terminal, Duration::from_secs(5));

        terminal.navigate(temp.path().join("does-not-exist"));
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if matches!(
                terminal.event_rx.recv_timeout(Duration::from_millis(100)),
                Ok(TerminalEvent::NavigationFailed)
            ) {
                return;
            }
        }
        panic!("Bash did not reject the missing directory");
    }

    #[test]
    fn scene_navigation_refreshes_the_visible_prompt_directory() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        let root = temp.path().join("root");
        let child = root.join("child");
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(&child).unwrap();
        fs::write(
            home.join(".bashrc"),
            b"PS1='PROMPT:\\w> '\nPROMPT_COMMAND=('printf \"PRECMD\\n\"')\n",
        )
        .unwrap();
        let shell = temp.path().join("test-bash");
        fs::write(
            &shell,
            format!(
                "#!/bin/bash\nexport HOME={}\nexec /bin/bash \"$@\"\n",
                home.display()
            ),
        )
        .unwrap();
        fs::set_permissions(&shell, fs::Permissions::from_mode(0o700)).unwrap();

        let terminal = Terminal::spawn_with_shell(&root, Some(shell.into_os_string())).unwrap();
        wait_for_prompt(&terminal, Duration::from_secs(5));
        terminal.send_input(b"printf 'BUFFER_PRESERVED\\n'".to_vec());
        terminal.navigate(child.clone());
        wait_for_cwd(&terminal, &child, Duration::from_secs(5));
        wait_for_prompt(&terminal, Duration::from_secs(5));
        thread::sleep(Duration::from_millis(50));

        let text = terminal
            .snapshot()
            .spans
            .iter()
            .map(|span| span.text.as_str())
            .collect::<String>();
        assert!(
            text.contains(&format!("PROMPT:{}> ", child.display())),
            "the visible prompt did not refresh after navigation: {text:?}"
        );
        assert!(
            text.contains("PRECMD"),
            "the existing PROMPT_COMMAND array did not run: {text:?}"
        );
        assert!(
            text.contains("printf 'BUFFER_PRESERVED\\n'"),
            "the pending command line was not restored: {text:?}"
        );

        terminal.send_input(b"\r".to_vec());
        wait_for_prompt(&terminal, Duration::from_secs(5));
        let text = terminal
            .snapshot()
            .spans
            .iter()
            .map(|span| span.text.as_str())
            .collect::<String>();
        assert!(text.contains("BUFFER_PRESERVED"));
    }

    fn wait_for_cwd(terminal: &Terminal, expected: &Path, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if let Ok(TerminalEvent::Cwd(path)) =
                terminal.event_rx.recv_timeout(Duration::from_millis(100))
                && path == expected
            {
                return;
            }
        }
        panic!("Bash did not report {}", expected.display());
    }

    fn wait_for_prompt(terminal: &Terminal, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if matches!(
                terminal.event_rx.recv_timeout(Duration::from_millis(100)),
                Ok(TerminalEvent::Prompt)
            ) {
                return;
            }
        }
        panic!("Bash prompt did not become ready");
    }
}
