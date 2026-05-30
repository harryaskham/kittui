//! ratakittui widget wrappers.
//!
//! Each wrapper holds an underlying ratatui widget plus a `Chrome` and
//! exposes a `render_with(area, buf, runtime) -> RenderEffects` method.
//! Implementations are uniform: produce chrome effects, then render the
//! inner widget into the post-padding rect. Wrappers are independent of
//! ratatui's `Widget` trait so they can also be invoked manually, but each
//! also implements `Widget` for `Frame::render_widget`-style ergonomics.
//! Note that `Widget::render` cannot produce effects; hosts using
//! `Frame::render_widget` must call `render_with` separately if they want
//! chrome. The recommended path is `draw_with_kittui`, which threads the
//! effects sink for you.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::widgets::canvas::Canvas;
use ratatui::widgets::{
    BarChart, Block, Chart, Clear, Gauge, LineGauge, List, Paragraph, Scrollbar, ScrollbarState,
    Sparkline, Table, Tabs, Widget,
};

use kittui::Runtime;

use crate::chrome::Chrome;
use crate::{render_chrome, RenderEffects};

/// Wrap any ratatui `Widget` with a kittui `Chrome`. The blanket wrapper
/// is what every named wrapper (`KittuiBlock`, `KittuiTable`, ...) reduces
/// to internally; users who want their own widget type can use this
/// directly.
pub struct KittuiDecorated<W> {
    /// The underlying ratatui widget.
    pub widget: W,
    /// kittui chrome to render under and around the widget.
    pub chrome: Chrome,
}

impl<W> KittuiDecorated<W> {
    /// Build a decorated widget.
    pub fn new(widget: W, chrome: Chrome) -> Self {
        Self { widget, chrome }
    }
}

impl<W: Widget> KittuiDecorated<W> {
    /// Render chrome and inner widget. Returns effects the host must flush.
    pub fn render_with(self, area: Rect, buf: &mut Buffer, runtime: &Runtime) -> RenderEffects {
        let effects = render_chrome(area, &self.chrome, runtime);
        let inner = self.chrome.inner_rect(area);
        self.widget.render(inner, buf);
        effects
    }
}

impl<W: Widget> Widget for KittuiDecorated<W> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let inner = self.chrome.inner_rect(area);
        self.widget.render(inner, buf);
    }
}

macro_rules! named_wrapper {
    ($(#[$meta:meta])* $name:ident, $inner:ty) => {
        $(#[$meta])*
        pub struct $name<'a> {
            /// Underlying ratatui widget.
            pub widget: $inner,
            /// kittui chrome.
            pub chrome: Chrome,
            _marker: std::marker::PhantomData<&'a ()>,
        }

        impl<'a> $name<'a> {
            /// Build a wrapped widget.
            pub fn new(widget: $inner, chrome: Chrome) -> Self {
                Self {
                    widget,
                    chrome,
                    _marker: std::marker::PhantomData,
                }
            }

            /// Render chrome and inner widget; return effects for the host.
            pub fn render_with(self, area: Rect, buf: &mut Buffer, runtime: &Runtime) -> RenderEffects {
                let effects = render_chrome(area, &self.chrome, runtime);
                let inner = self.chrome.inner_rect(area);
                self.widget.render(inner, buf);
                effects
            }
        }

        impl<'a> Widget for $name<'a> {
            fn render(self, area: Rect, buf: &mut Buffer) {
                let inner = self.chrome.inner_rect(area);
                self.widget.render(inner, buf);
            }
        }
    };
}

named_wrapper!(
    /// kittui-decorated [`ratatui::widgets::Block`].
    KittuiBlock,
    Block<'a>
);
named_wrapper!(
    /// kittui-decorated [`ratatui::widgets::Paragraph`].
    KittuiParagraph,
    Paragraph<'a>
);
named_wrapper!(
    /// kittui-decorated [`ratatui::widgets::List`].
    KittuiList,
    List<'a>
);
named_wrapper!(
    /// kittui-decorated [`ratatui::widgets::Table`].
    KittuiTable,
    Table<'a>
);
named_wrapper!(
    /// kittui-decorated [`ratatui::widgets::Tabs`].
    KittuiTabs,
    Tabs<'a>
);
named_wrapper!(
    /// kittui-decorated [`ratatui::widgets::Gauge`].
    KittuiGauge,
    Gauge<'a>
);
named_wrapper!(
    /// kittui-decorated [`ratatui::widgets::LineGauge`].
    KittuiLineGauge,
    LineGauge<'a>
);
named_wrapper!(
    /// kittui-decorated [`ratatui::widgets::Sparkline`].
    KittuiSparkline,
    Sparkline<'a>
);
named_wrapper!(
    /// kittui-decorated [`ratatui::widgets::BarChart`].
    KittuiBarChart,
    BarChart<'a>
);
named_wrapper!(
    /// kittui-decorated [`ratatui::widgets::Chart`].
    KittuiChart,
    Chart<'a>
);

/// kittui-decorated [`ratatui::widgets::Scrollbar`]. Scrollbar is a
/// `StatefulWidget`, so this wrapper takes `&mut ScrollbarState` at render
/// time rather than implementing `Widget`.
pub struct KittuiScrollbar<'a> {
    /// Underlying ratatui widget.
    pub widget: Scrollbar<'a>,
    /// kittui chrome.
    pub chrome: Chrome,
}

impl<'a> KittuiScrollbar<'a> {
    /// Build a wrapped scrollbar.
    pub fn new(widget: Scrollbar<'a>, chrome: Chrome) -> Self {
        Self { widget, chrome }
    }

    /// Render chrome and the inner scrollbar; return effects for the host.
    pub fn render_with(
        self,
        area: Rect,
        buf: &mut Buffer,
        state: &mut ScrollbarState,
        runtime: &Runtime,
    ) -> RenderEffects {
        use ratatui::widgets::StatefulWidget;
        let effects = render_chrome(area, &self.chrome, runtime);
        let inner = self.chrome.inner_rect(area);
        self.widget.render(inner, buf, state);
        effects
    }
}

/// kittui-decorated [`ratatui::widgets::Clear`]. `Clear` is not lifetime-
/// bound so it gets its own wrapper without the `'a` parameter.
pub struct KittuiClear {
    /// Underlying ratatui widget.
    pub widget: Clear,
    /// kittui chrome.
    pub chrome: Chrome,
}

impl KittuiClear {
    /// Build a wrapped clear widget.
    pub fn new(chrome: Chrome) -> Self {
        Self {
            widget: Clear,
            chrome,
        }
    }

    /// Render chrome and inner widget; return effects for the host.
    pub fn render_with(self, area: Rect, buf: &mut Buffer, runtime: &Runtime) -> RenderEffects {
        let effects = render_chrome(area, &self.chrome, runtime);
        let inner = self.chrome.inner_rect(area);
        self.widget.render(inner, buf);
        effects
    }
}

impl Widget for KittuiClear {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let inner = self.chrome.inner_rect(area);
        self.widget.render(inner, buf);
    }
}

/// kittui-decorated [`ratatui::widgets::canvas::Canvas`]. `Canvas` is
/// generic over its painter closure so it doesn't fit the `named_wrapper!`
/// macro; this is a manual implementation.
pub struct KittuiCanvas<'a, F>
where
    F: Fn(&mut ratatui::widgets::canvas::Context<'_>),
{
    /// Underlying ratatui canvas.
    pub widget: Canvas<'a, F>,
    /// kittui chrome.
    pub chrome: Chrome,
}

impl<'a, F> KittuiCanvas<'a, F>
where
    F: Fn(&mut ratatui::widgets::canvas::Context<'_>),
{
    /// Build a wrapped canvas.
    pub fn new(widget: Canvas<'a, F>, chrome: Chrome) -> Self {
        Self { widget, chrome }
    }

    /// Render chrome and inner widget; return effects for the host.
    pub fn render_with(self, area: Rect, buf: &mut Buffer, runtime: &Runtime) -> RenderEffects {
        let effects = render_chrome(area, &self.chrome, runtime);
        let inner = self.chrome.inner_rect(area);
        self.widget.render(inner, buf);
        effects
    }
}

impl<'a, F> Widget for KittuiCanvas<'a, F>
where
    F: Fn(&mut ratatui::widgets::canvas::Context<'_>),
{
    fn render(self, area: Rect, buf: &mut Buffer) {
        let inner = self.chrome.inner_rect(area);
        self.widget.render(inner, buf);
    }
}

/// Inline single-line widgets. These are chrome-only; the "content" is
/// the chrome itself plus optional text drawn into the host buffer.
pub mod inline {
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
    use ratatui::style::Style;
    use ratatui::text::Line;
    use ratatui::widgets::{Paragraph, Widget};

    use kittui::Runtime;

    use crate::chrome::Chrome;
    use crate::{render_chrome, RenderEffects};

    fn render_text(area: Rect, buf: &mut Buffer, text: &str, style: Style) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        Paragraph::new(Line::styled(text.to_owned(), style)).render(area, buf);
    }

    /// Single-line chip with chrome + label.
    pub struct KittuiChip<'a> {
        /// Label text. Drawn into the chrome's inner rect.
        pub label: &'a str,
        /// Chrome decoration.
        pub chrome: Chrome,
        /// Text style for the label.
        pub style: Style,
    }

    impl<'a> KittuiChip<'a> {
        /// Construct a chip.
        pub fn new(label: &'a str, chrome: Chrome) -> Self {
            Self {
                label,
                chrome,
                style: Style::default(),
            }
        }

        /// Builder helper to override the label style.
        pub fn style(mut self, style: Style) -> Self {
            self.style = style;
            self
        }

        /// Render and return effects.
        pub fn render_with(
            self,
            area: Rect,
            buf: &mut Buffer,
            runtime: &Runtime,
        ) -> RenderEffects {
            let effects = render_chrome(area, &self.chrome, runtime);
            let inner = self.chrome.inner_rect(area);
            render_text(inner, buf, self.label, self.style);
            effects
        }
    }

    /// Single-line title bar (no inner widget; just chrome + text).
    pub struct KittuiTitle<'a> {
        /// Title text.
        pub label: &'a str,
        /// Chrome decoration.
        pub chrome: Chrome,
        /// Text style.
        pub style: Style,
    }

    impl<'a> KittuiTitle<'a> {
        /// Construct a title.
        pub fn new(label: &'a str, chrome: Chrome) -> Self {
            Self {
                label,
                chrome,
                style: Style::default(),
            }
        }

        /// Render and return effects.
        pub fn render_with(
            self,
            area: Rect,
            buf: &mut Buffer,
            runtime: &Runtime,
        ) -> RenderEffects {
            let effects = render_chrome(area, &self.chrome, runtime);
            let inner = self.chrome.inner_rect(area);
            render_text(inner, buf, self.label, self.style);
            effects
        }
    }

    /// Horizontal divider: a chromed one-line strip with no content.
    pub struct KittuiDivider {
        /// Chrome decoration.
        pub chrome: Chrome,
    }

    impl KittuiDivider {
        /// Construct a divider.
        pub fn new(chrome: Chrome) -> Self {
            Self { chrome }
        }

        /// Render and return effects.
        pub fn render_with(self, area: Rect, _buf: &mut Buffer, runtime: &Runtime) -> RenderEffects {
            render_chrome(area, &self.chrome, runtime)
        }
    }

    /// A chromed line with arbitrary text content.
    pub struct KittuiLine<'a> {
        /// Inline line content.
        pub line: Line<'a>,
        /// Chrome decoration.
        pub chrome: Chrome,
    }

    impl<'a> KittuiLine<'a> {
        /// Construct a chromed line.
        pub fn new(line: Line<'a>, chrome: Chrome) -> Self {
            Self { line, chrome }
        }

        /// Render and return effects.
        pub fn render_with(
            self,
            area: Rect,
            buf: &mut Buffer,
            runtime: &Runtime,
        ) -> RenderEffects {
            let effects = render_chrome(area, &self.chrome, runtime);
            let inner = self.chrome.inner_rect(area);
            if inner.width > 0 && inner.height > 0 {
                Paragraph::new(self.line).render(inner, buf);
            }
            effects
        }
    }
}

pub use inline::{KittuiChip, KittuiDivider, KittuiLine, KittuiTitle};

#[cfg(test)]
mod tests {
    use super::KittuiBlock;
    use crate::chrome::{Background, Border, Chrome};
    use ratatui::layout::Rect;
    use kittui::{RendererKind, Runtime};
    use ratatui::buffer::Buffer;
    use ratatui::widgets::{Block, Borders};
    use std::fmt::Write as FmtWrite;

    fn tempdir() -> std::path::PathBuf {
        let pid = std::process::id();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(ratakittui_test_temp_dir_name(pid, nanos));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn ratakittui_test_temp_dir_name(pid: u32, nanos: u128) -> String {
        let mut name = String::with_capacity(
            "ratakittui-".len() + decimal_len(pid as u128) + 1 + decimal_len(nanos),
        );
        name.push_str("ratakittui-");
        write!(name, "{pid}-{nanos}").expect("write to string");
        name
    }

    fn decimal_len(mut value: u128) -> usize {
        let mut digits = 1;
        while value >= 10 {
            value /= 10;
            digits += 1;
        }
        digits
    }

    fn runtime() -> Runtime {
        Runtime::builder()
            .cache_dir(tempdir())
            .renderer(RendererKind::Cpu)
            .build()
            .unwrap()
    }

    #[test]
    fn ratakittui_test_temp_dir_name_builds_directly() {
        let name = ratakittui_test_temp_dir_name(1234, 5678);
        assert_eq!(name, "ratakittui-1234-5678");
        assert_eq!(name.capacity(), name.len());
        assert_eq!(decimal_len(0), 1);
        assert_eq!(decimal_len(9), 1);
        assert_eq!(decimal_len(10), 2);
    }

    #[test]
    fn decorated_block_emits_effects_and_writes_into_inner_buffer() {
        let runtime = runtime();
        let area = Rect::new(0, 0, 12, 4);
        let mut buf = Buffer::empty(area);
        let chrome = Chrome::default()
            .background(Background::Solid(kittui::Rgba::rgb(0, 0xd8, 0xff)))
            .border(Border::rounded(kittui::Rgba::rgb(0xff, 0xff, 0xff), 1.0, 4.0));
        let wrapper = KittuiBlock::new(Block::default().borders(Borders::NONE), chrome);
        let effects = wrapper.render_with(area, &mut buf, &runtime);
        assert!(!effects.is_empty());
        assert!(effects.image_id.is_some());
        assert_eq!(effects.footprint, Some(kittui::CellRect::new(0, 0, 12, 4)));
    }
}
