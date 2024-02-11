// glyph module code along with comments was originally retired from glyph-brush
// https://github.com/alexheretic/glyph-brush
// glyph-brush was originally written Alex Butler (https://github.com/alexheretic)
// and licensed under Apache-2.0 license.

mod brush;
mod cache;
mod calculator;
mod extra;
mod layout;
mod section;

pub use crate::glyph::{brush::*, calculator::*, extra::*, section::*};
pub use cache::Rectangle;
pub use layout::*;

use layout::ab_glyph::*;

/// A "practically collision free" `Section` hasher
#[cfg(not(target_arch = "wasm32"))]
pub type DefaultSectionHasher = twox_hash::RandomXxHashBuilder;
// Work around for rand issues in wasm #61
#[cfg(target_arch = "wasm32")]
pub type DefaultSectionHasher = std::hash::BuildHasherDefault<twox_hash::XxHash>;

#[test]
fn default_section_hasher() {
    use std::hash::{BuildHasher, Hash, Hasher};

    let section_a = Section::default().add_text(Text::new("Hovered Tile: Some((0, 0))"));
    let section_b = Section::default().add_text(Text::new("Hovered Tile: Some((1, 0))"));
    let hash = |s: &Section| {
        let mut hasher = DefaultSectionHasher::default().build_hasher();
        s.hash(&mut hasher);
        hasher.finish()
    };
    assert_ne!(hash(&section_a), hash(&section_b));
}
