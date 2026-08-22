//! Fabushi self-hosted messaging domain.
//!
//! This crate is intentionally independent from Telegram APIs and MTProto.
//! Telegram Desktop and Unigram are compatibility/UX references only; humans,
//! AI assistants, bots, services, payments, wallets, Mini Apps, realtime calls,
//! media, self-hosted blob storage, communities, stories, devices, search,
//! privacy, notifications, scoped access, networking, signaling, and durable
//! sync share one Fabushi-owned protocol and state machine.

pub mod access;
pub mod actor;
pub mod blob_store;
pub mod bot;
pub mod community;
pub mod conversation;
pub mod device;
pub mod engine;
pub mod gateway;
pub mod media;
pub mod message;
pub mod miniapp;
pub mod notification;
pub mod payment;
pub mod payment_provider;
pub mod privacy;
pub mod protocol;
pub mod realtime;
pub mod search;
pub mod secret_chat;
pub mod server;
pub mod service;
pub mod settlement;
pub mod signaling;
pub mod signaling_server;
pub mod store;
pub mod story;
pub mod wallet;

pub use access::*;
pub use actor::*;
pub use blob_store::*;
pub use bot::*;
pub use community::*;
pub use conversation::*;
pub use device::*;
pub use engine::{Command, EngineError, Event, MessagingEngine, MessagingState};
pub use gateway::*;
pub use media::*;
pub use message::*;
pub use miniapp::*;
pub use notification::*;
pub use payment::*;
pub use payment_provider::*;
pub use privacy::*;
pub use protocol::*;
pub use realtime::*;
pub use search::*;
pub use secret_chat::*;
pub use server::*;
pub use service::*;
pub use settlement::*;
pub use signaling::*;
pub use signaling_server::*;
pub use store::*;
pub use story::*;
pub use wallet::*;
