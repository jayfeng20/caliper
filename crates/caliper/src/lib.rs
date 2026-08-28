//! caliper — structured terminal review of agent-generated PR findings.
//!
//! The pipeline is deliberately one-directional at its core: review agents emit a
//! report as JSON, caliper owns every presentation decision, and the reviewer's
//! triage is written back into the *same* document so the round trip is idempotent
//! and diffable.

pub mod render;
pub mod report;
