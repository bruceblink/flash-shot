//! Platform-independent product concepts.

pub mod annotation;
pub mod geometry;
mod roadmap;
pub mod selection;
pub mod session;

pub use roadmap::{Capability, DeliveryPhase, ProductRoadmap};
