//! Judgment core: every decision of the app and every data format it reads, with no outside
//! communication. It reads no files, opens no connections, runs no timers and knows neither the VPN
//! client's name nor any sentence the user sees. Given the same inputs in the same order it gives
//! the same results, so the runtime feeds it every input and carries out what it decides.

pub mod bypass_rules;
pub mod config;
pub mod connection;
pub mod gmail_api;
pub mod imap;
pub mod oauth;
pub mod otp_mail;
