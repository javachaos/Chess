use std::cell::RefCell;

use crate::ui_app::ChessApp;
use wasm_bindgen::prelude::*;

thread_local! {
    static RUNNER: RefCell<Option<eframe::WebRunner>> = const { RefCell::new(None) };
}

#[wasm_bindgen]
pub async fn start_app(canvas: eframe::web_sys::HtmlCanvasElement) -> Result<(), JsValue> {
    destroy_app();

    let runner = eframe::WebRunner::new();
    runner
        .start(
            canvas,
            eframe::WebOptions::default(),
            Box::new(|cc| Ok(Box::new(ChessApp::new(cc)))),
        )
        .await?;

    RUNNER.with(|slot| {
        *slot.borrow_mut() = Some(runner);
    });

    Ok(())
}

#[wasm_bindgen]
pub fn destroy_app() {
    RUNNER.with(|slot| {
        if let Some(runner) = slot.borrow_mut().take() {
            runner.destroy();
        }
    });
}

#[wasm_bindgen]
pub fn app_has_panicked() -> bool {
    RUNNER.with(|slot| {
        slot.borrow()
            .as_ref()
            .is_some_and(eframe::WebRunner::has_panicked)
    })
}

#[wasm_bindgen]
pub fn app_panic_message() -> Option<String> {
    RUNNER.with(|slot| {
        slot.borrow()
            .as_ref()
            .and_then(|runner| runner.panic_summary().map(|summary| summary.message()))
    })
}

#[wasm_bindgen]
pub fn app_panic_callstack() -> Option<String> {
    RUNNER.with(|slot| {
        slot.borrow()
            .as_ref()
            .and_then(|runner| runner.panic_summary().map(|summary| summary.callstack()))
    })
}
