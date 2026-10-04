//! ae's own ratatui `Backend`: cursor moves and SGR to a writer, the ONE place
//! `ae app` writes an escape sequence. A cell's symbol is checked before it is
//! written, so no text a record carries can reach the terminal as a control.

use std::io::{self, Write};

use ratatui_core::backend::{Backend, ClearType, WindowSize};
use ratatui_core::buffer::Cell;
use ratatui_core::layout::{Position, Size};
use ratatui_core::style::{Color, Modifier};

/// Cells to `out`, at the size the terminal last reported.
pub(crate) struct Ansi<W: Write> {
    out: W,
    size: Size,
}

impl<W: Write> Ansi<W> {
    pub(crate) fn new(out: W, size: (u16, u16)) -> Self {
        Self {
            out,
            size: Size::new(size.0, size.1),
        }
    }

    /// The size the next frame is drawn at.
    pub(crate) fn resize(&mut self, size: (u16, u16)) {
        self.size = Size::new(size.0, size.1);
    }
}

/// The SGR parameters that set `cell`'s whole look from a reset.
fn sgr(cell: &Cell) -> String {
    let mut params = vec!["0".to_owned()];
    for (modifier, code) in [
        (Modifier::BOLD, "1"),
        (Modifier::DIM, "2"),
        (Modifier::ITALIC, "3"),
        (Modifier::UNDERLINED, "4"),
        (Modifier::REVERSED, "7"),
    ] {
        if cell.modifier.contains(modifier) {
            params.push(code.to_owned());
        }
    }
    for (colour, ground) in [(cell.fg, 38), (cell.bg, 48)] {
        match colour {
            Color::Rgb(red, green, blue) => params.push(format!("{ground};2;{red};{green};{blue}")),
            Color::Indexed(index) => params.push(format!("{ground};5;{index}")),
            _ => {}
        }
    }
    format!("\x1b[{}m", params.join(";"))
}

impl<W: Write> Backend for Ansi<W> {
    type Error = io::Error;

    fn draw<'a, I>(&mut self, content: I) -> io::Result<()>
    where
        I: Iterator<Item = (u16, u16, &'a Cell)>,
    {
        let mut at: Option<(u16, u16)> = None;
        let mut look = String::new();
        for (x, y, cell) in content {
            if at != Some((x, y)) {
                write!(self.out, "\x1b[{};{}H", y + 1, x + 1)?;
            }
            let wanted = sgr(cell);
            if wanted != look {
                self.out.write_all(wanted.as_bytes())?;
                look = wanted;
            }
            let symbol = cell.symbol();
            let clean = if symbol.chars().any(char::is_control) {
                " "
            } else {
                symbol
            };
            self.out.write_all(clean.as_bytes())?;
            // A wide symbol moves the terminal's cursor past more than one
            // cell, so the next cell is always placed explicitly.
            let wide = ratatui_core::text::Span::raw(clean).width() != 1;
            at = (!wide).then_some((x + 1, y));
        }
        self.out.write_all(b"\x1b[0m")
    }

    fn hide_cursor(&mut self) -> io::Result<()> {
        self.out.write_all(b"\x1b[?25l")
    }

    fn show_cursor(&mut self) -> io::Result<()> {
        self.out.write_all(b"\x1b[?25h")
    }

    fn get_cursor_position(&mut self) -> io::Result<Position> {
        Ok(Position::ORIGIN)
    }

    fn set_cursor_position<P: Into<Position>>(&mut self, position: P) -> io::Result<()> {
        let position = position.into();
        write!(self.out, "\x1b[{};{}H", position.y + 1, position.x + 1)
    }

    fn clear(&mut self) -> io::Result<()> {
        self.out.write_all(b"\x1b[0m\x1b[2J")
    }

    fn clear_region(&mut self, clear_type: ClearType) -> io::Result<()> {
        let code: &[u8] = match clear_type {
            ClearType::All => b"\x1b[2J",
            ClearType::AfterCursor => b"\x1b[J",
            ClearType::BeforeCursor => b"\x1b[1J",
            ClearType::CurrentLine => b"\x1b[2K",
            ClearType::UntilNewLine => b"\x1b[K",
        };
        self.out.write_all(code)
    }

    fn size(&self) -> io::Result<Size> {
        Ok(self.size)
    }

    fn window_size(&mut self) -> io::Result<WindowSize> {
        Ok(WindowSize {
            columns_rows: self.size,
            pixels: Size::default(),
        })
    }

    fn flush(&mut self) -> io::Result<()> {
        self.out.flush()
    }
}

#[cfg(test)]
mod tests {
    //! Oracle: ECMA-48 — CUP is `CSI row ; column H`, both 1-based; SGR
    //! `38;2;r;g;b` / `48;2;r;g;b` set a direct colour, `0` resets.

    use super::{Ansi, Backend as _};
    use ratatui_core::buffer::Cell;
    use ratatui_core::style::{Color, Modifier};

    fn cell(symbol: &str) -> Cell {
        let mut cell = Cell::default();
        cell.set_symbol(symbol);
        cell
    }

    fn drawn(cells: &[(u16, u16, Cell)]) -> String {
        let mut out = Vec::new();
        Ansi::new(&mut out, (10, 2))
            .draw(cells.iter().map(|(x, y, cell)| (*x, *y, cell)))
            .expect("a Vec takes every byte");
        String::from_utf8(out).expect("the backend writes UTF-8")
    }

    #[test]
    fn a_run_is_placed_once_and_the_frame_ends_reset() {
        let text = drawn(&[(2, 1, cell("a")), (3, 1, cell("b")), (7, 0, cell("c"))]);
        assert_eq!(text, "\x1b[2;3H\x1b[0mab\x1b[1;8Hc\x1b[0m");
    }

    #[test]
    fn a_look_is_written_whole_and_only_when_it_changes() {
        let mut bold = cell("x");
        bold.set_fg(Color::Rgb(1, 2, 3))
            .set_bg(Color::Rgb(4, 5, 6))
            .modifier = Modifier::BOLD | Modifier::REVERSED;
        let text = drawn(&[(0, 0, bold.clone()), (1, 0, bold), (2, 0, cell("y"))]);
        assert_eq!(
            text,
            "\x1b[1;1H\x1b[0;1;7;38;2;1;2;3;48;2;4;5;6mxx\x1b[0my\x1b[0m"
        );
    }

    #[test]
    fn a_control_symbol_reaches_the_terminal_as_a_blank() {
        for control in ["\x1b", "\x07", "\u{9b}", "a\x1b[2J"] {
            let text = drawn(&[(0, 0, cell(control))]);
            assert_eq!(text, "\x1b[1;1H\x1b[0m \x1b[0m", "{control:?}");
        }
    }

    #[test]
    fn the_cell_after_a_wide_symbol_is_placed_explicitly() {
        // Even the column right after it: the terminal's cursor is already
        // two columns on, so only an explicit move lands the next symbol.
        let text = drawn(&[(0, 0, cell("你")), (1, 0, cell("x"))]);
        assert_eq!(text, "\x1b[1;1H\x1b[0m你\x1b[1;2Hx\x1b[0m");
    }

    // ---- mutation pins (pins-plan.md #72/#75-79/#86-89). Oracles: ECMA-48
    // ---- (CUP, ED, EL, SGR 38;5 / 48;5), DECTCEM and the Backend contract.

    fn written(act: impl FnOnce(&mut Ansi<&mut Vec<u8>>) -> std::io::Result<()>) -> String {
        let mut out = Vec::new();
        act(&mut Ansi::new(&mut out, (10, 2))).expect("a Vec takes every byte");
        String::from_utf8(out).expect("the backend writes UTF-8")
    }

    #[test]
    fn a_resize_is_the_size_the_next_frame_is_drawn_at() {
        use ratatui_core::layout::Size;
        let mut ansi = Ansi::new(Vec::new(), (160, 45));
        assert_eq!(ansi.size().ok(), Some(Size::new(160, 45)), "as reported");
        ansi.resize((100, 30));
        assert_eq!(ansi.size().ok(), Some(Size::new(100, 30)), "after a resize");
    }

    #[test]
    fn an_indexed_colour_is_written_as_sgr_5() {
        let mut indexed = cell("x");
        indexed
            .set_fg(Color::Indexed(208))
            .set_bg(Color::Indexed(17));
        let text = drawn(&[(0, 0, indexed)]);
        assert!(text.contains(";38;5;208"), "{text:?}");
        assert!(text.contains(";48;5;17"), "{text:?}");
    }

    #[test]
    fn the_cursor_controls_are_the_standard_sequences() {
        assert_eq!(written(|ansi| ansi.hide_cursor()), "\x1b[?25l");
        assert_eq!(written(|ansi| ansi.show_cursor()), "\x1b[?25h");
        // CUP is 1-based: column 4, row 2 is `ESC[3;5H`.
        assert_eq!(
            written(|ansi| ansi.set_cursor_position((4, 2))),
            "\x1b[3;5H"
        );
    }

    #[test]
    fn the_erase_controls_are_the_standard_sequences() {
        use ratatui_core::backend::ClearType;
        assert!(written(|ansi| ansi.clear()).ends_with("\x1b[2J"));
        for (kind, sequence) in [
            (ClearType::All, "\x1b[2J"),
            (ClearType::AfterCursor, "\x1b[J"),
            (ClearType::BeforeCursor, "\x1b[1J"),
            (ClearType::CurrentLine, "\x1b[2K"),
            (ClearType::UntilNewLine, "\x1b[K"),
        ] {
            assert_eq!(
                written(|ansi| ansi.clear_region(kind)),
                sequence,
                "{kind:?}"
            );
        }
    }

    /// Counts the flushes that reach it.
    #[derive(Default)]
    struct Recorder {
        flushed: usize,
    }

    impl std::io::Write for Recorder {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            self.flushed += 1;
            Ok(())
        }
    }

    #[test]
    fn a_flush_reaches_the_writer() {
        let mut recorder = Recorder::default();
        Ansi::new(&mut recorder, (10, 2))
            .flush()
            .expect("the recorder flushes");
        assert_eq!(recorder.flushed, 1);
    }
}
