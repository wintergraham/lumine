//! Native Linux/macOS application host with GPU rendering, accessibility, clipboard,
//! and IME. A software raster backend is available for reference and headless tests.
//!
//! ```no_run
//! use lumine::compose::prelude::*;
//! use lumine_desktop::{Application, WindowOptions};
//! fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     Application::new().window(WindowOptions {
//!         title: "Example".into(), ..Default::default()
//!     }).run(|cx| {
//!         cx.render(component(|cx| {
//!             let title = cx.state(String::from("Ready"));
//!             let displayed = title.clone();
//!             column().p(24.0).gap(12.0)
//!                 .child(text_signal(move || displayed.get()))
//!                 .child(button().w(140.0).h(36.0).child("Update")
//!                     .on_click(move || { title.set(String::from("Updated")); }))
//!         }));
//!     })
//! }
//! ```
mod clipboard;
mod focus_memory;
mod native_ime;
pub mod presentation;
pub mod raster;

pub mod accessibility;

pub mod application;
pub use application::{
    Application, TaskSpawner, WindowContext, WindowFactory, WindowHandle, WindowOptions,
};

mod scoped_task;
pub use scoped_task::ScopedTask;

#[cfg(doctest)]
#[doc = include_str!("../../../README.md")]
mod readme_examples {}

mod presentation_retry;

#[cfg(doctest)]
#[doc = include_str!("../../../docs/window-controls.md")]
mod window_control_examples {}
