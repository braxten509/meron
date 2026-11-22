//! Regression suites for the mail engine.
//! Each module owns the fixtures for one part of synchronization or delivery.
//! Transport tests use local fixtures and cache tests use an in-memory database.

mod folder_roles;
mod folder_tail;
mod pool;
mod prefetch_policy;
mod search_pages;
mod sent_copies;
mod transfer_recovery;
