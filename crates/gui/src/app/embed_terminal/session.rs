use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use alacritty_terminal::event::{Event, EventListener, WindowSize};
use alacritty_terminal::event_loop::{EventLoop, EventLoopSender, Msg};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::{Term, term, tty};
use gpui_kit::*;
use settings::config::terminal::TerminalCursorShape;

use super::{ActivePreferences, terminal_background};

#[derive(Clone)]
pub(super) struct TerminalEvents {
    sender: async_channel::Sender<Event>,
    wake_pending: Arc<AtomicBool>,
}

impl EventListener for TerminalEvents {
    fn send_event(&self, event: Event) {
        if matches!(event, Event::Wakeup) && self.wake_pending.swap(true, Ordering::Relaxed) {
            return;
        }
        let _ = self.sender.try_send(event);
    }
}

pub(super) struct Session {
    pub grid: Arc<FairMutex<Term<TerminalEvents>>>,
    pub status: Option<String>,
    pub title: SharedString,
    sender: EventLoopSender,
    size: PtySize,
    cursor_shape: TerminalCursorShape,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct PtySize {
    pub columns: u16,
    pub rows: u16,
    pub cell_width: u16,
    pub cell_height: u16,
}

impl Dimensions for PtySize {
    fn total_lines(&self) -> usize {
        self.screen_lines()
    }

    fn screen_lines(&self) -> usize {
        self.rows.into()
    }

    fn columns(&self) -> usize {
        self.columns.into()
    }
}

impl From<PtySize> for WindowSize {
    fn from(size: PtySize) -> Self {
        Self {
            num_lines: size.rows,
            num_cols: size.columns,
            cell_width: size.cell_width,
            cell_height: size.cell_height,
        }
    }
}

impl Session {
    pub fn start(
        directory: Option<PathBuf>,
        cx: &mut Context<super::TerminalView>,
    ) -> anyhow::Result<Self> {
        let options = tty::Options {
            working_directory: directory,
            shell: std::env::var("SHELL")
                .ok()
                .filter(|shell| !shell.is_empty())
                .map(|shell| tty::Shell::new(shell, Vec::new())),
            env: [
                ("TERM".into(), "xterm-256color".into()),
                ("COLORTERM".into(), "truecolor".into()),
            ]
            .into(),
            ..Default::default()
        };
        let cursor_shape = cx.global::<ActivePreferences>().0.terminal.cursor_shape;
        let (session, receiver, wake_pending) = Self::launch(&options, cursor_shape)?;
        cx.spawn(async move |view, cx| {
            while let Ok(event) = receiver.recv().await {
                if matches!(event, Event::Wakeup) {
                    wake_pending.store(false, Ordering::Relaxed);
                }
                if view
                    .update(cx, |view, cx| {
                        if let Some(session) = view.session.as_mut() {
                            session.handle_event(event, cx);
                            cx.notify();
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();

        Ok(session)
    }

    fn launch(
        options: &tty::Options,
        cursor_shape: TerminalCursorShape,
    ) -> anyhow::Result<(Self, async_channel::Receiver<Event>, Arc<AtomicBool>)> {
        let size = PtySize {
            columns: 80,
            rows: 16,
            cell_width: 8,
            cell_height: 18,
        };
        let (sender, receiver) = async_channel::unbounded();
        let wake_pending = Arc::new(AtomicBool::new(false));
        let events = TerminalEvents {
            sender,
            wake_pending: wake_pending.clone(),
        };
        let grid = Arc::new(FairMutex::new(Term::new(
            terminal_options(cursor_shape),
            &size,
            events.clone(),
        )));
        let pty = tty::new(options, size.into(), 0)?;
        let event_loop = EventLoop::new(grid.clone(), events, pty, true, false)?;
        let sender = event_loop.channel();

        // 分离 I/O 线程。线程退出时释放 PTY，避免在界面线程等待 shell。
        drop(event_loop.spawn());

        let session = Self {
            grid,
            status: None,
            title: "终端".into(),
            sender,
            size,
            cursor_shape,
        };
        Ok((session, receiver, wake_pending))
    }

    pub fn set_cursor_shape(&mut self, cursor_shape: TerminalCursorShape) {
        if self.cursor_shape != cursor_shape {
            self.cursor_shape = cursor_shape;
            self.grid.lock().set_options(terminal_options(cursor_shape));
        }
    }

    pub fn resize(&mut self, size: PtySize) {
        if size != self.size {
            self.size = size;
            self.grid.lock().resize(size);
            let _ = self.sender.send(Msg::Resize(size.into()));
        }
    }

    pub fn write(&self, bytes: impl Into<Vec<u8>>) {
        let _ = self.sender.send(Msg::Input(bytes.into().into()));
    }

    fn handle_event(&mut self, event: Event, cx: &App) {
        match event {
            Event::Title(title) if !title.is_empty() => self.title = title.into(),
            Event::ResetTitle | Event::Title(_) => self.title = "终端".into(),
            Event::PtyWrite(text) => self.write(text),
            Event::TextAreaSizeRequest(format) => self.write(format(self.size.into())),
            Event::ColorRequest(index, format) => {
                let background =
                    terminal_background(&cx.global::<ActivePreferences>().0.terminal.background);
                let color = super::display::palette(index, self.grid.lock().colors(), background);
                self.write(format(color));
            }
            Event::ChildExit(status) => {
                self.status = Some(format!("会话已退出（{status}）"));
            }
            Event::Exit => {
                self.status.get_or_insert_with(|| "会话已退出".into());
            }
            _ => {}
        }
    }
}

fn terminal_options(shape: TerminalCursorShape) -> term::Config {
    use alacritty_terminal::vte::ansi::{CursorShape, CursorStyle};

    term::Config {
        default_cursor_style: CursorStyle {
            shape: match shape {
                TerminalCursorShape::Block => CursorShape::Block,
                TerminalCursorShape::Beam => CursorShape::Beam,
                TerminalCursorShape::Underline => CursorShape::Underline,
            },
            blinking: false,
        },
        ..Default::default()
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.sender.send(Msg::Shutdown);
    }
}

#[cfg(all(test, unix))]
mod tests {
    use std::time::{Duration, Instant};

    use alacritty_terminal::index::Side;
    use alacritty_terminal::selection::{Selection, SelectionType};
    use alacritty_terminal::vte::ansi::{Color, CursorShape, CursorStyle, Handler, NamedColor};

    use super::{Event, PtySize, Session, TerminalCursorShape, tty};

    #[test]
    fn independent_ptys_use_their_own_cwd_input_and_size() {
        fn launch(
            path: &str,
            rows: u16,
            columns: u16,
            text: &str,
        ) -> (Session, async_channel::Receiver<Event>) {
            let options = tty::Options {
                shell: Some(tty::Shell::new(
                    "/bin/sh".into(),
                    vec![
                        "-c".into(),
                        r#"stty -echo; IFS= read -r message; printf '\033[32m%s\033[0m\n' "$message"; pwd; stty size"#.into(),
                    ],
                )),
                working_directory: Some(path.into()),
                ..Default::default()
            };
            let (mut session, events, _) =
                Session::launch(&options, TerminalCursorShape::default()).unwrap();
            session.resize(PtySize {
                rows,
                columns,
                ..session.size
            });
            session.write(format!("{text}\r"));
            (session, events)
        }

        fn output(session: &Session, events: async_channel::Receiver<Event>) -> (String, String) {
            let deadline = Instant::now() + Duration::from_secs(4);
            loop {
                if matches!(events.try_recv(), Ok(Event::Exit)) {
                    break;
                }
                assert!(Instant::now() < deadline, "等待终端退出超时");
                std::thread::sleep(Duration::from_millis(5));
            }

            let mut grid = session.grid.lock();
            let mut screen = String::new();
            let mut green = None;
            for cell in grid.renderable_content().display_iter {
                if cell.point.column.0 == 0 {
                    screen.push('\n');
                }
                screen.push(cell.c);
                if cell.fg == Color::Named(NamedColor::Green) {
                    let (start, end) = green.get_or_insert((cell.point, cell.point));
                    assert_eq!(start.line, cell.point.line);
                    *end = cell.point;
                }
            }
            let (start, end) = green.expect("找不到 ANSI 着色的文本");
            let mut selection = Selection::new(SelectionType::Simple, start, Side::Left);
            selection.update(end, Side::Right);
            grid.selection = Some(selection);
            (screen, grid.selection_to_string().unwrap())
        }

        let (mut first, first_events) = launch("/", 11, 50, "first");
        first.set_cursor_shape(TerminalCursorShape::Beam);
        assert_eq!(first.grid.lock().cursor_style().shape, CursorShape::Beam);
        first.grid.lock().set_cursor_style(Some(CursorStyle {
            shape: CursorShape::Block,
            blinking: false,
        }));
        first.set_cursor_shape(TerminalCursorShape::Underline);
        assert_eq!(first.grid.lock().cursor_style().shape, CursorShape::Block);
        first.grid.lock().set_cursor_style(None);
        assert_eq!(
            first.grid.lock().cursor_style().shape,
            CursorShape::Underline
        );
        let temp = std::fs::canonicalize("/tmp").unwrap();
        let temp = temp.to_str().unwrap();
        let (second, second_events) = launch(temp, 13, 60, "second");
        let (first_screen, first_green) = output(&first, first_events);
        let (second_screen, second_green) = output(&second, second_events);
        assert_eq!(first_green, "first");
        assert_eq!(second_green, "second");
        assert!(first_screen.lines().any(|line| line.trim_end() == "/"));
        assert!(second_screen.lines().any(|line| line.trim_end() == temp));
        assert!(first_screen.contains("11 50"));
        assert!(second_screen.contains("13 60"));
    }
}
