//! Exercises session reuse in the engine connection pool.
//! Returning a session observes the capacity bound, while taking one observes idle time.
//! The tests use integer session handles and explicit timestamps.

use crate::engine::{Pooled, pool_return, pool_take};
use std::collections::HashMap;
use std::time::{Duration, Instant};

const MAX_IDLE: Duration = Duration::from_secs(120);
const MAX_POOLED: usize = 3;

#[test]
fn take_returns_none_for_unknown_account() {
    let mut map: HashMap<String, Vec<Pooled<u32>>> = HashMap::new();
    assert_eq!(pool_take(&mut map, "a", Instant::now(), MAX_IDLE), None);
}

#[test]
fn return_then_take_round_trips_lifo() {
    let mut map: HashMap<String, Vec<Pooled<u32>>> = HashMap::new();
    let now = Instant::now();
    pool_return(&mut map, "a", 1, now, MAX_POOLED);
    pool_return(&mut map, "a", 2, now, MAX_POOLED);
    // LIFO: the hottest (last returned) comes back first.
    assert_eq!(pool_take(&mut map, "a", now, MAX_IDLE), Some(2));
    assert_eq!(pool_take(&mut map, "a", now, MAX_IDLE), Some(1));
    assert_eq!(pool_take(&mut map, "a", now, MAX_IDLE), None);
}

#[test]
fn return_drops_session_when_at_capacity() {
    let mut map: HashMap<String, Vec<Pooled<u32>>> = HashMap::new();
    let now = Instant::now();
    for session in 0..(MAX_POOLED as u32 + 2) {
        pool_return(&mut map, "a", session, now, MAX_POOLED);
    }
    assert_eq!(map["a"].len(), MAX_POOLED);
}

#[test]
fn take_evicts_sessions_idle_past_max_idle() {
    let mut map: HashMap<String, Vec<Pooled<u32>>> = HashMap::new();
    let now = Instant::now();
    let stale = now.checked_sub(MAX_IDLE + Duration::from_secs(1)).unwrap();
    // One stale entry, then a fresh one on top.
    pool_return(&mut map, "a", 1, stale, MAX_POOLED);
    pool_return(&mut map, "a", 2, now, MAX_POOLED);
    // Fresh one is taken; the stale one underneath is evicted on the next take.
    assert_eq!(pool_take(&mut map, "a", now, MAX_IDLE), Some(2));
    assert_eq!(pool_take(&mut map, "a", now, MAX_IDLE), None);
}
