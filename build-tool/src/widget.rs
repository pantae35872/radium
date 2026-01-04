use ratatui::{
    buffer::Buffer,
    layout::Rect,
    text::Text,
    widgets::{Block, Paragraph, Widget, Wrap},
};

pub mod config_area;
pub mod prompt;

#[derive(Default)]
pub struct CenteredParagraph<'a> {
    text: Text<'a>,
    block: Option<Block<'a>>,
}

impl<'a> CenteredParagraph<'a> {
    pub fn new<T: Into<Text<'a>>>(text: T) -> Self {
        Self { text: text.into(), block: None }
    }

    pub fn block(mut self, block: Block<'a>) -> Self {
        self.block = Some(block);
        self
    }
}

impl<'a> Widget for CenteredParagraph<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let (pad_x, pad_y) = if let Some(ref block) = self.block {
            let inner = block.inner(area);
            (area.width.saturating_sub(inner.width), area.height.saturating_sub(inner.height))
        } else {
            (0, 0)
        };

        let max_width = area.width.saturating_sub(pad_x);

        let (text_w, text_h) = measure_text(&self.text, max_width);

        let widget_width = text_w + pad_x;
        let widget_height = text_h + pad_y;

        let rect = Rect {
            x: area.x + (area.width.saturating_sub(widget_width)) / 2,
            y: area.y + (area.height.saturating_sub(widget_height)) / 2,
            width: widget_width,
            height: widget_height,
        };

        let mut paragraph = Paragraph::new(self.text).wrap(Wrap { trim: false });

        if let Some(block) = self.block {
            paragraph = paragraph.block(block);
        }

        for x in rect.left()..rect.right() {
            for y in rect.top()..rect.bottom() {
                buf[(x, y)].set_fg(ratatui::style::Color::White);
            }
        }

        paragraph.render(rect, buf);
    }
}
fn measure_text(text: &Text, max_width: u16) -> (u16, u16) {
    let mut width = 0;
    let mut height = 0;

    for line in &text.lines {
        let line_width = line.width() as u16;
        let wrapped = if line_width == 0 { 1 } else { (line_width + max_width - 1) / max_width };

        height += wrapped;
        width = width.max(line_width.min(max_width));
    }

    (width, height)
}
