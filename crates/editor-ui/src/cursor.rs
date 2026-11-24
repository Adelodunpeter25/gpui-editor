//! Reusable caret blink timer for text inputs (`SearchBar`, etc.).

use gpui::{Context, Task};
use std::time::Duration;

const BLINK_INTERVAL: Duration = Duration::from_millis(530);

pub(crate) struct CursorBlink {
    visible: bool,
    task: Option<Task<()>>,
}

impl Default for CursorBlink {
    fn default() -> Self {
        Self {
            visible: true,
            task: None,
        }
    }
}

impl CursorBlink {
    pub(crate) fn is_visible(&self) -> bool {
        self.visible
    }

    pub(crate) fn set_visible(&mut self, visible: bool) {
        self.visible = visible;
    }

    pub(crate) fn reset<T: 'static>(
        &mut self,
        cx: &mut Context<T>,
        mut on_toggle: impl FnMut(&mut T, bool, &mut Context<T>) + 'static,
    ) {
        self.visible = true;
        let executor = cx.background_executor().clone();
        self.task = Some(cx.spawn(async move |this, cx| {
            let mut visible = true;
            loop {
                executor.timer(BLINK_INTERVAL).await;
                visible = !visible;
                let keep_going = this
                    .update(cx, |entity, cx| {
                        on_toggle(entity, visible, cx);
                        cx.notify();
                    })
                    .is_ok();
                if !keep_going {
                    break;
                }
            }
        }));
    }

    pub(crate) fn stop(&mut self) {
        self.task = None;
        self.visible = true;
    }
}
