//! LightAnim → overlay::CoreAnimSpec 的桥接(后续 Phase 扩展用)。
//! 现仅 re-export,保证模块地图完整。
#![allow(dead_code, unused_imports)]

pub use crate::overlay::{AnimKind, CoreAnimSpec};

pub fn spec_from(a: agent_light_core::LightAnim) -> CoreAnimSpec {
    CoreAnimSpec::from(a)
}
