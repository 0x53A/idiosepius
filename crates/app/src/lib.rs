//! WASM entry point: register Idiosepius as a reusable custom element.
//!
//! The native executable remains in `main.rs`. The browser build follows the
//! `web-component-rs` pattern and mounts the ordinary egui application into a
//! shadow-root canvas owned by `<idiosepius-app>`.

#[cfg(target_arch = "wasm32")]
mod app;
#[cfg(target_arch = "wasm32")]
mod background;
#[cfg(target_arch = "wasm32")]
mod blocks;
#[cfg(target_arch = "wasm32")]
mod browser;
#[cfg(target_arch = "wasm32")]
mod card;
#[cfg(target_arch = "wasm32")]
mod coin;
#[cfg(target_arch = "wasm32")]
mod explain;
#[cfg(target_arch = "wasm32")]
mod import;
#[cfg(target_arch = "wasm32")]
mod import_dialog;
#[cfg(target_arch = "wasm32")]
mod math;
#[cfg(target_arch = "wasm32")]
mod plot;
#[cfg(target_arch = "wasm32")]
mod richtext;
#[cfg(target_arch = "wasm32")]
mod settings;
#[cfg(target_arch = "wasm32")]
mod theme;

#[cfg(target_arch = "wasm32")]
mod component {
    use egui_web_component::EguiMount;
    use rust_web_component::WebComponent;
    use rust_web_component_macro::WebComponent;
    use wasm_bindgen_futures::spawn_local;

    use crate::browser::{BrowserApp, InitialState};

    #[derive(WebComponent)]
    #[web_component(name = "idiosepius-app")]
    pub struct IdiosepiusComponent {
        element: Option<web_sys::HtmlElement>,
        mount: Option<EguiMount>,
    }

    impl IdiosepiusComponent {
        fn new() -> Self {
            let _ = eframe::WebLogger::init(log::LevelFilter::Info);
            Self {
                element: None,
                mount: None,
            }
        }
    }

    impl WebComponent for IdiosepiusComponent {
        fn attach(&mut self, element: &web_sys::HtmlElement) {
            self.element = Some(element.clone());
        }

        fn connected(&mut self) {
            let Some(element) = self.element.clone() else {
                return;
            };
            let component_element = element.clone();
            spawn_local(async move {
                let initial = InitialState::load().await;
                let result = EguiMount::connect(
                    &element,
                    eframe::WebOptions::default(),
                    Box::new(move |creation| {
                        Ok(Box::new(BrowserApp::new(&creation.egui_ctx, initial)))
                    }),
                )
                .await;

                match result {
                    Ok(mount) => {
                        IdiosepiusComponent::with_element(&component_element, |component| {
                            component.mount = Some(mount);
                        });
                    }
                    Err(error) => web_sys::console::error_1(&error),
                }
            });
        }

        fn disconnected(&mut self) {
            if let Some(mount) = self.mount.take() {
                mount.disconnect();
            }
        }
    }
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn start() {
    console_error_panic_hook::set_once();
    component::IdiosepiusComponent::setup();
}
