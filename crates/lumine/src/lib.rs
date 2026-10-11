//! Fine-grained reactive state and a retained, damage-tracked scene.
//! The core has no platform dependencies and can be targeted by template compilers.
pub mod collections;
pub mod reactive;
pub mod scene;
pub mod task;
pub mod view;
pub mod virtual_list;

pub mod input;
pub mod semantics;
pub mod text_edit;

pub mod widgets;

pub mod keyed;

pub mod image;

pub mod components;

pub mod timer;

pub mod effects;
pub mod frame;
pub mod motion;

pub mod rich_text;
pub mod text_layout;

pub mod affine;
pub mod background;

pub mod compose;
mod compose_menu;
mod compose_menu_scroll;
mod compose_modal;
mod compose_popover;
mod compose_rich;
mod compose_scrollbar;
mod compose_style;
mod compose_virtual_keyboard;
pub mod style;

#[cfg(doctest)]
#[doc = include_str!("../../../docs/composition.md")]
mod composition_guide {}
#[cfg(doctest)]
#[doc = include_str!("../../../docs/styling.md")]
mod styling_guide {}
#[cfg(doctest)]
#[doc = include_str!("../../../docs/motion.md")]
mod motion_guide {}

pub mod actions;

mod action_context;

pub mod canvas;

pub mod animation;

pub mod layout;

pub mod image_cache;

pub mod decoration;

pub mod svg;

pub mod cursor;

mod compose_drag;

#[cfg(target_os = "macos")]
pub mod native_surface;
