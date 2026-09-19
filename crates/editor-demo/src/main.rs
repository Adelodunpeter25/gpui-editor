mod types;

use gpui::prelude::FluentBuilder;
use gpui::*;
use std::borrow::Cow;
use types::DemoApp;

static FONT_JETBRAINS_MONO_REGULAR: &[u8] =
    include_bytes!("../assets/fonts/JetBrainsMono-Regular.ttf");

actions!(demo, [OpenFile, Quit, ToggleWrap, ToggleDiff, ToggleFont]);

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

        let wrap_enabled = self.state.read(cx).wrap_enabled();
        let showing_diff = self.showing_diff;
        let using_custom_font = self.using_custom_font;
        let diff = self.diff_state.read(cx);
        let diff_label = if diff.is_empty() {
            "Diff (no changes)".to_string()
        } else {
            format!("Diff (+{} -{})", diff.added(), diff.removed())
        };

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
            .on_action(cx.listener(|_this, _: &Quit, _window, cx| {
                cx.quit();
            }))
            .on_action(cx.listener(|this, _: &ToggleWrap, _window, cx| {
                this.toggle_wrap(cx);
            }))
            .on_action(cx.listener(|this, _: &ToggleDiff, _window, cx| {
                this.toggle_diff(cx);
            }))
            .on_action(cx.listener(|this, _: &ToggleFont, _window, cx| {
                this.toggle_font(cx);
            }))
            // Top toolbar / status bar with Open button
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .px_3()
                    .py_2()
                    .bg(rgb(0x181825))
                    .border_b_1()
                    .border_color(rgb(0x313244))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_3()
                            .child(
                                div()
                                    .id("open-button")
                                    .px_3()
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
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .id("wrap-toggle")
                                    .px_2()
                                    .py_0p5()
                                    .rounded_md()
                                    .bg(if wrap_enabled {
                                        rgb(0x89b4fa)
                                    } else {
                                        rgb(0x313244)
                                    })
                                    .hover(|s| s.bg(rgb(0x45475a)))
                                    .cursor_pointer()
                                    .text_xs()
                                    .text_color(if wrap_enabled {
                                        rgb(0x1e1e2e)
                                    } else {
                                        rgb(0xcdd6f4)
                                    })
                                    .child("Wrap")
                                    .on_click(cx.listener(|this, _, _window, cx| {
                                        this.toggle_wrap(cx);
                                    })),
                            )
                            .child(
                                div()
                                    .id("diff-toggle")
                                    .px_2()
                                    .py_0p5()
                                    .rounded_md()
                                    .bg(if showing_diff {
                                        rgb(0x89b4fa)
                                    } else {
                                        rgb(0x313244)
                                    })
                                    .hover(|s| s.bg(rgb(0x45475a)))
                                    .cursor_pointer()
                                    .text_xs()
                                    .text_color(if showing_diff {
                                        rgb(0x1e1e2e)
                                    } else {
                                        rgb(0xcdd6f4)
                                    })
                                    .child(diff_label)
                                    .on_click(cx.listener(|this, _, _window, cx| {
                                        this.toggle_diff(cx);
                                    })),
                            )
                            .child(
                                div()
                                    .id("font-toggle")
                                    .px_2()
                                    .py_0p5()
                                    .rounded_md()
                                    .bg(if using_custom_font {
                                        rgb(0x89b4fa)
                                    } else {
                                        rgb(0x313244)
                                    })
                                    .hover(|s| s.bg(rgb(0x45475a)))
                                    .cursor_pointer()
                                    .text_xs()
                                    .text_color(if using_custom_font {
                                        rgb(0x1e1e2e)
                                    } else {
                                        rgb(0xcdd6f4)
                                    })
                                    .child(if using_custom_font {
                                        "Font: Menlo"
                                    } else {
                                        "Font: JetBrains Mono"
                                    })
                                    .on_click(cx.listener(|this, _, _window, cx| {
                                        this.toggle_font(cx);
                                    })),
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
                    ),
            )
            // Editor main area — swaps to the diff view while toggled on.
            .child(div().flex_1().size_full().map(|el| {
                if showing_diff {
                    el.child(self.diff_view.clone())
                } else {
                    el.child(self.view.clone())
                }
            }))
    }
}

fn main() {
    let platform = gpui_platform::current_platform(false);
    Application::with_platform(platform).run(|cx: &mut App| {
        if let Err(error) = cx
            .text_system()
            .add_fonts(vec![Cow::Borrowed(FONT_JETBRAINS_MONO_REGULAR)])
        {
            eprintln!("Failed to register bundled fonts: {error}");
        }

        // Configure macOS Application Menus so the app name and menus appear in the menu bar
        cx.set_menus(vec![
            Menu {
                name: "GPUI Editor".into(),
                items: vec![
                    MenuItem::action("Quit GPUI Editor", Quit),
                ],
                disabled: false,
            },
            Menu {
                name: "File".into(),
                items: vec![
                    MenuItem::action("Open File…", OpenFile),
                ],
                disabled: false,
            },
            Menu {
                name: "View".into(),
                items: vec![
                    MenuItem::action("Toggle Word Wrap", ToggleWrap),
                    MenuItem::action("Toggle Diff View", ToggleDiff),
                    MenuItem::action("Toggle Font (Menlo)", ToggleFont),
                ],
                disabled: false,
            },
        ]);

        cx.bind_keys([
            KeyBinding::new("cmd-o", OpenFile, Some("DemoApp")),
            KeyBinding::new("ctrl-o", OpenFile, Some("DemoApp")),
            KeyBinding::new("cmd-q", Quit, None),
            KeyBinding::new("cmd-alt-z", ToggleWrap, Some("DemoApp")),
            KeyBinding::new("ctrl-alt-z", ToggleWrap, Some("DemoApp")),
            KeyBinding::new("cmd-alt-d", ToggleDiff, Some("DemoApp")),
            KeyBinding::new("ctrl-alt-d", ToggleDiff, Some("DemoApp")),
            KeyBinding::new("cmd-alt-f", ToggleFont, Some("DemoApp")),
            KeyBinding::new("ctrl-alt-f", ToggleFont, Some("DemoApp")),
        ]);
        editor_ui::init(cx);

        cx.open_window(
            WindowOptions {
                titlebar: Some(TitlebarOptions {
                    title: Some("GPUI Editor".into()),
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

        cx.activate(true);
    });
}
