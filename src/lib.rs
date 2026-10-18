#![allow(non_snake_case)]
#![deny(unsafe_op_in_unsafe_fn)]

pub mod appcast;
mod crypto;
mod ffi;
pub mod http;
mod installer;
mod platform;
mod settings;
mod ui;
mod updater;

type Result<T> = std::result::Result<T, String>;
