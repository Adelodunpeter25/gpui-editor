use editor_ui::{EditorState, EditorView};
use gpui::*;

const SAMPLE: &str = r#"// gpui-editor M0 — readonly viewer (raw gpui 0.2.2)
fn main() {
    let greeting = "hello gpui"; // string + comment
    let answer = 42;
    println!("{greeting} {answer}");
}

struct Buffer {
    version: u64,
}

enum Mode {
    ReadOnly,
    Editable,
}
"#;

struct DemoApp {
    view: Entity<EditorView>,
}

impl DemoApp {
    fn new(cx: &mut Context<Self>) -> Self {
        let state =
            cx.new(|_| EditorState::readonly(SAMPLE, syntax::LanguageRegistry::for_name("rust")));
        let view = cx.new(|_| EditorView::new(&state));
        Self { view }
    }
}

impl Render for DemoApp {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(self.view.clone())
    }
}

fn main() {
    let platform = gpui_platform::current_platform(false);
    Application::with_platform(platform).run(|cx: &mut App| {
        cx.open_window(WindowOptions::default(), |_window, cx| {
            cx.new(DemoApp::new)
        })
        .expect("failed to open window");
    });
}
