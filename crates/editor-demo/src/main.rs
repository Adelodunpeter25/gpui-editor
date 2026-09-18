use editor_ui::{EditorState, EditorView};
use gpui::*;
use std::fs;
use std::path::PathBuf;
use syntax::LanguageRegistry;

actions!(demo, [OpenFile]);

const INITIAL_SAMPLE: &str = r#"// Press Cmd+O (or Ctrl+O) to open any file in Finder / file dialog!
// Syntax highlighting is powered by Tree-sitter & Lumis across all languages.

fn main() {
    let greeting = "Hello, GPUI Editor!";
    let answer = 42;
    println!("{greeting} - The answer is {answer}");
}

pub struct State {
    pub loaded: bool,
}
"#;

struct DemoApp {
    state: Entity<EditorState>,
    view: Entity<EditorView>,
    file_path: Option<PathBuf>,
    focus_handle: FocusHandle,
}

impl DemoApp {
    fn new(cx: &mut Context<Self>) -> Self {
        let state = cx.new(|_| {
            let registry = LanguageRegistry::builtin();
            EditorState::readonly(INITIAL_SAMPLE, registry.for_name("rust"))
        });
        let view = cx.new(|_| EditorView::new(&state));
        let focus_handle = cx.focus_handle();

        Self {
            state,
            view,
            file_path: None,
            focus_handle,
        }
    }

    fn open_file(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if let Some(path) = rfd::FileDialog::new().pick_file() {
            if let Ok(content) = fs::read_to_string(&path) {
                let registry = LanguageRegistry::builtin();
                let lang = LanguageRegistry::for_path(&path).or_else(|| {
                    path.extension()
                        .and_then(|ext| ext.to_str())
                        .and_then(|ext| registry.for_extension(ext))
                });

                self.state.update(cx, |editor, _cx| {
                    editor.set_text(&content);
                    editor.set_language(lang);
                });

                self.file_path = Some(path);
                cx.notify();
            }
        }
    }
}

impl Render for DemoApp {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let path_text = self
            .file_path
            .as_ref()
            .and_then(|p| p.to_str())
            .unwrap_or("Demo buffer (Press Cmd+O to open file)");

        let lang_name = self
            .state
            .read(cx)
            .language()
            .map(|l| l.name)
            .unwrap_or("plain text");

        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(0x1e1e2e))
            .track_focus(&self.focus_handle)
            .key_context("DemoApp")
            .on_action(cx.listener(|this, _: &OpenFile, window, cx| {
                this.open_file(window, cx);
            }))
            // Top toolbar / status bar with Open button
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .px_3()
                    .py_1p5()
                    .bg(rgb(0x181825))
                    .border_b_1()
                    .border_color(rgb(0x313244))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .id("open-button")
                                    .px_2p5()
                                    .py_1()
                                    .rounded_md()
                                    .bg(rgb(0x313244))
                                    .hover(|s| s.bg(rgb(0x45475a)))
                                    .cursor_pointer()
                                    .text_xs()
                                    .text_color(rgb(0xcdd6f4))
                                    .child("📁 Open File (Cmd+O)")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.open_file(window, cx);
                                    })),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(0xa6adc8))
                                    .child(path_text.to_string()),
                            ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .px_2()
                            .py_0p5()
                            .rounded_md()
                            .bg(rgb(0x313244))
                            .text_color(rgb(0x89b4fa))
                            .child(format!("Language: {lang_name}")),
                    ),
            )
            // Editor main area
            .child(div().flex_1().size_full().child(self.view.clone()))
    }
}

fn main() {
    let platform = gpui_platform::current_platform(false);
    Application::with_platform(platform).run(|cx: &mut App| {
        cx.bind_keys([
            KeyBinding::new("cmd-o", OpenFile, Some("DemoApp")),
            KeyBinding::new("ctrl-o", OpenFile, Some("DemoApp")),
        ]);

        cx.open_window(
            WindowOptions {
                titlebar: Some(TitlebarOptions {
                    title: Some("GPUI Editor Demo".into()),
                    appears_transparent: false,
                    traffic_light_position: None,
                }),
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(1000.0), px(700.0)),
                    cx,
                ))),
                ..Default::default()
            },
            |_window, cx| cx.new(DemoApp::new),
        )
        .expect("failed to open window");
    });
}
