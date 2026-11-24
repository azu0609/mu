use crate::{
    remote::Remote,
    text::{Line, Style},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use crossterm::{style::Color, terminal};
use std::{
    cell::RefCell,
    collections::HashMap,
    env,
    io::{self, Write},
    path::PathBuf,
};

// Images go through kitty's Unicode placeholders: plain text cells the
// terminal paints the image over, so they scroll and clip like any line.
const PLACEHOLDER: char = '\u{10EEEE}';
const DIACRITICS: [char; 24] = [
    '\u{0305}', '\u{030D}', '\u{030E}', '\u{0310}', '\u{0312}', '\u{033D}', '\u{033E}', '\u{033F}', '\u{0346}',
    '\u{034A}', '\u{034B}', '\u{034C}', '\u{0350}', '\u{0351}', '\u{0352}', '\u{0357}', '\u{035B}', '\u{0363}',
    '\u{0364}', '\u{0365}', '\u{0366}', '\u{0367}', '\u{0368}', '\u{0369}',
];

// The terminal keeps an image once sent, so it is only held until then.
enum Data {
    Unsent(String),
    Sent,
    Gone,
    Asked,
}

struct Png {
    id: u32,
    width: f64,
    height: f64,
    data: Data,
    size: Option<(usize, usize)>,
    placed: Option<(usize, usize)>,
}

impl Png {
    fn new(id: u32, base64: String) -> Option<Self> {
        let header = STANDARD.decode(base64.get(..32)?).ok()?;
        if !header.starts_with(b"\x89PNG\r\n\x1a\n") {
            return None;
        }
        let dimension = |at: usize| u32::from_be_bytes(header[at..at + 4].try_into().unwrap()) as f64;
        let (width, height) = (dimension(16), dimension(20));
        Some(Self { id, width, height, data: Data::Unsent(base64), size: None, placed: None })
    }

    fn cells(&self, width: usize, (cell_width, cell_height): (f64, f64)) -> (usize, usize) {
        let scale =
            1f64.min(width as f64 * cell_width / self.width).min(DIACRITICS.len() as f64 * cell_height / self.height);
        let fit = |pixels: f64, cell: f64, limit: usize| ((pixels * scale / cell).ceil() as usize).clamp(1, limit);
        (fit(self.width, cell_width, width.max(1)), fit(self.height, cell_height, DIACRITICS.len()))
    }

    fn transmit(&self, out: &mut impl Write, base64: &str) -> io::Result<()> {
        let mut control = format!("a=t,f=100,q=2,i={},", self.id);
        let mut chunks = base64.as_bytes().chunks(4096).peekable();
        while let Some(chunk) = chunks.next() {
            write!(out, "\x1b_G{control}m={};", u8::from(chunks.peek().is_some()))?;
            out.write_all(chunk)?;
            out.write_all(b"\x1b\\")?;
            control.clear();
        }
        Ok(())
    }
}

enum State {
    Wanted,
    Loading,
    Failed,
    Ready(Png),
}

pub struct Images {
    supported: bool,
    states: RefCell<HashMap<PathBuf, State>>,
    ids: u32,
    cell: (f64, f64),
}

impl Images {
    pub fn new() -> Self {
        let supported = matches!(env::var("TERM").as_deref(), Ok("xterm-kitty" | "xterm-ghostty"));
        Self { supported, states: RefCell::default(), ids: 0, cell: (8.0, 16.0) }
    }

    pub fn measure(&mut self) {
        if let Ok(size) = terminal::window_size()
            && self.supported
            && size.width > 0
            && size.height > 0
        {
            self.cell = (size.width as f64 / size.columns as f64, size.height as f64 / size.rows as f64);
        }
    }

    pub fn lines(&self, path: PathBuf, width: usize) -> Vec<Line> {
        if !self.supported {
            return vec![];
        }
        let mut states = self.states.borrow_mut();
        let State::Ready(png) = states.entry(path).or_insert(State::Wanted) else {
            return vec![];
        };
        let (columns, rows) = png.cells(width, self.cell);
        png.size = Some((columns, rows));
        let id = png.id.to_be_bytes();
        let style = Style::new(Color::Rgb { r: id[1], g: id[2], b: id[3] });
        let rest = PLACEHOLDER.to_string().repeat(columns - 1);
        (0..rows)
            .map(|row| Line::styled(format!("{PLACEHOLDER}{}{}{rest}", DIACRITICS[row], DIACRITICS[0]), style))
            .collect()
    }

    pub fn loaded(&mut self, path: PathBuf, base64: crate::Result<String>) {
        let states = self.states.get_mut();
        match (states.get_mut(&path), base64) {
            (Some(State::Ready(png)), Ok(base64)) => png.data = Data::Unsent(base64),
            (_, base64) => {
                self.ids += 1;
                let png = base64.ok().and_then(|base64| Png::new(self.ids, base64));
                states.insert(path, png.map_or(State::Failed, State::Ready));
            }
        }
    }

    // Asks the host for the images drawn that the terminal lacks, and sends
    // the terminal those that arrived and the size each is drawn at.
    pub fn flush(&mut self, remote: &Remote, out: &mut impl Write) -> io::Result<()> {
        for (path, state) in self.states.get_mut() {
            match state {
                State::Wanted => {
                    remote.read_image(path.clone());
                    *state = State::Loading;
                }
                State::Ready(png) => {
                    let Some((columns, rows)) = png.size else { continue };
                    match &png.data {
                        Data::Asked => continue,
                        Data::Gone => {
                            remote.read_image(path.clone());
                            png.data = Data::Asked;
                            continue;
                        }
                        Data::Unsent(base64) => {
                            png.transmit(out, base64)?;
                            png.data = Data::Sent;
                            png.placed = None;
                        }
                        Data::Sent => (),
                    }
                    if png.placed != png.size {
                        write!(out, "\x1b_Ga=p,U=1,q=2,i={},p=1,c={columns},r={rows}\x1b\\", png.id)?;
                        png.placed = png.size;
                    }
                }
                _ => (),
            }
        }
        Ok(())
    }

    // The terminal drops its images with the screen they were on.
    pub fn forget(&mut self) {
        for state in self.states.get_mut().values_mut() {
            if let State::Ready(png) = state {
                if let Data::Sent = png.data {
                    png.data = Data::Gone;
                }
                png.size = None;
                png.placed = None;
            }
        }
    }
}
