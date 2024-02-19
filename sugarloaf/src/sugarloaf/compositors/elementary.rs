// Copyright (c) 2023-present, Raphael Amorim.
//
// This source code is licensed under the MIT license found in the
// LICENSE file in the root directory of this source tree.

use crate::sugarloaf::TextArea;
use crate::text::Text;
use crate::components::text::glyph::{FontId, GlyphCruncher, OwnedSection};
use crate::font::{
    FONT_ID_BOLD, FONT_ID_BOLD_ITALIC, FONT_ID_EMOJIS, FONT_ID_ICONS, FONT_ID_ITALIC,
    FONT_ID_REGULAR, FONT_ID_SYMBOL, FONT_ID_UNICODE,
};
use crate::sugarloaf::graphics;
use crate::sugarloaf::primitives::fragment::*;
use crate::sugarloaf::text;
use crate::sugarloaf::tree::SugarTree;
use crate::sugarloaf::{PxScale, Rect};
use ab_glyph::{Font, FontArc};
use fnv::FnvHashMap;
use unicode_width::UnicodeWidthChar;

#[derive(Copy, Clone, PartialEq)]
pub struct CachedSugar {
    font_id: FontId,
    char_width: f32,
    px_scale: Option<PxScale>,
}

#[allow(unused)]
struct GraphicRect {
    id: graphics::SugarGraphicId,
    height: u16,
    width: u16,
    pos_x: f32,
    pos_y: f32,
    columns: f32,
    start_row: f32,
    end_row: f32,
}

#[derive(Default)]
pub struct Elementary {
    sugar_cache: FnvHashMap<char, CachedSugar>,
    pub rects: Vec<Rect>,
    pub sections: Vec<OwnedSection>,
    pub text_sections: Vec<OwnedSection>,
    pub should_resize: bool,
    text_y: f32,
    current_row: u16,
    fonts: Vec<FontArc>,
    graphic_rects: FnvHashMap<crate::SugarGraphicId, GraphicRect>,
}

impl Elementary {
    #[inline]
    pub fn set_fonts(&mut self, fonts: Vec<FontArc>) {
        self.fonts = fonts
    }

    #[inline]
    pub fn rects(&mut self) -> &Vec<Rect> {
        &self.rects
    }

    #[inline]
    pub fn clean_blocks(&mut self) {
        self.text_sections.clear();
    }

    #[inline]
    pub fn set_should_resize(&mut self) {
        self.should_resize = true;
    }

    #[inline]
    pub fn calculate_dimensions(
        &mut self,
        content: char,
        font_id: usize,
        tree: &SugarTree,
        brush: &mut text::GlyphBrush<()>,
    ) -> (f32, f32) {
        let text = crate::components::text::Text {
            text: &content.to_owned().to_string(),
            scale: PxScale {
                x: tree.layout.style.text_scale,
                y: tree.layout.style.text_scale,
            },
            font_id: crate::sugarloaf::FontId(font_id),
            extra: crate::components::text::Extra {
                color: [0., 0., 0., 0.],
                z: 0.0,
            },
        };

        let section = &crate::components::text::Section {
            screen_position: (0., 0.),
            bounds: (tree.layout.width, tree.layout.height),
            text: vec![text],
            layout: crate::components::text::glyph::Layout::default_single_line()
                .v_align(crate::components::text::glyph::VerticalAlign::Bottom)
                .h_align(crate::components::text::glyph::HorizontalAlign::Left),
        };

        brush.queue(section);

        if let Some(rect) = brush.glyph_bounds(section) {
            let width = rect.max.x - rect.min.x;
            let height = rect.max.y - rect.min.y;
            return (width, height);
        }

        (0., 0.)
    }

    #[inline]
    pub fn get_font_id(&mut self, sugar: &Fragment, tree: &SugarTree) -> CachedSugar {
        if let Some(cached_sugar) = self.sugar_cache.get(&sugar.content) {
            return *cached_sugar;
        }

        // #[allow(clippy::unnecessary_to_owned)]
        // let fonts: &[FontArc] = &self.text_brush.fonts().to_owned();
        let mut font_id: FontId = FontId(FONT_ID_REGULAR);

        for (idx, _font_arc) in self.fonts.iter().enumerate() {
            let found_glyph_id = self.fonts[idx].glyph_id(sugar.content);
            if found_glyph_id != ab_glyph::GlyphId(0) {
                font_id = FontId(idx);
                break;
            }
        }

        let mut px_scale = None;
        let char_width = sugar.content.width().unwrap_or(1) as f32;

        match font_id {
            // Icons will look for width 1
            FontId(FONT_ID_ICONS) => {
                px_scale = Some(PxScale {
                    x: tree.layout.dimensions.width,
                    y: tree.layout.dimensions.height,
                });
            }

            FontId(FONT_ID_UNICODE) | FontId(FONT_ID_SYMBOL) => {
                // println!("FONT_ID_UNICODE {:?}", char_width);
                px_scale = Some(PxScale {
                    x: tree.layout.dimensions.width * char_width,
                    y: tree.layout.dimensions.height,
                });
            }

            FontId(FONT_ID_EMOJIS) => {
                // scale_target = (tree.layout.dimensions.width * tree.layout.dimensions.scale) * 2.0;
                px_scale = Some(PxScale {
                    x: tree.layout.dimensions.width * 2.0,
                    y: tree.layout.dimensions.height,
                });
            }

            // FontId(FONT_ID_REGULAR) => {
            // px_scale = Some(PxScale {
            //     x: tree.layout.dimensions.width * 2.0,
            //     y: tree.layout.dimensions.height,
            // })
            // }
            FontId(_) => {}
        }

        let cached_sugar = CachedSugar {
            font_id,
            char_width,
            px_scale,
        };

        self.sugar_cache.insert(
            sugar.content,
            CachedSugar {
                font_id,
                char_width,
                px_scale,
            },
        );

        cached_sugar
    }

    #[inline]
    pub fn reset(&mut self) {
        // Clean font cache per instance
        self.sugar_cache.clear();
        self.clean();
    }

    #[inline]
    pub fn clean(&mut self) {
        self.rects.clear();
        self.sections.clear();
        self.graphic_rects.clear();
        self.current_row = 0;
        self.text_y = 0.0;
        self.should_resize = false;
    }

    #[inline]
    pub fn update(&mut self, line: usize, text_area: &TextArea, tree: &SugarTree) {
        let line = &text_area.inner[line];
        let mut x = 0.;
        let mod_pos_y = tree.layout.style.screen_position.1;
        let mod_text_y = tree.layout.dimensions.height;

        let sugar_x = tree.layout.dimensions.width;
        let sugar_width =
            (tree.layout.dimensions.width * tree.layout.dimensions.scale) * 2.;
        let unscaled_sugar_height =
            tree.layout.dimensions.height / tree.layout.dimensions.scale;

        if self.text_y == 0.0 {
            self.text_y = tree.layout.style.screen_position.1;
        }

        for i in 0..line.acc {
            let mut add_pos_x = sugar_x;
            let mut sugar_char_width = 1.;
            let rect_pos_x = tree.layout.style.screen_position.0 + x;

            let cached_sugar: CachedSugar = self.get_font_id(&line[i], tree);

            let mut font_id = cached_sugar.font_id;
            if cached_sugar.font_id == FontId(FONT_ID_REGULAR) {
                if line[i].style.is_bold_italic {
                    font_id = FontId(FONT_ID_BOLD_ITALIC);
                } else if line[i].style.is_bold {
                    font_id = FontId(FONT_ID_BOLD);
                } else if line[i].style.is_italic {
                    font_id = FontId(FONT_ID_ITALIC);
                }
            }

            if cached_sugar.char_width > 1. {
                sugar_char_width = cached_sugar.char_width;
                add_pos_x += sugar_x;
            }

            let mut scale = PxScale {
                x: tree.layout.dimensions.height,
                y: tree.layout.dimensions.height,
            };
            if let Some(new_scale) = cached_sugar.px_scale {
                scale = new_scale;
            }

            let rect_pos_y = self.text_y + mod_pos_y;
            let width_bound = sugar_width * sugar_char_width;

            let text = if line[i].repeated == 0 {
                line[i].content.to_string()
            } else {
                std::iter::repeat(line[i].content)
                    .take(line[i].repeated + 1)
                    .collect::<String>()
            };
            let section_pos_x = rect_pos_x;
            let section_pos_y = mod_text_y + self.text_y + mod_pos_y;

            let text = crate::components::text::OwnedText {
                text,
                scale,
                font_id,
                extra: crate::components::text::Extra {
                    color: line[i].foreground_color,
                    z: 0.0,
                },
            };

            self.sections.push(OwnedSection {
                screen_position: (section_pos_x, section_pos_y),
                bounds: (tree.layout.width, tree.layout.height),
                text: vec![text],
                layout: crate::components::text::glyph::Layout::default_single_line()
                    .v_align(crate::components::text::glyph::VerticalAlign::Bottom)
                    .h_align(crate::components::text::glyph::HorizontalAlign::Left),
            });

            // self.text_brush.queue(&section);

            let scaled_rect_pos_x = section_pos_x / tree.layout.dimensions.scale;
            let scaled_rect_pos_y = rect_pos_y / tree.layout.dimensions.scale;

            let quantity = (line[i].repeated + 1) as f32;

            self.rects.push(Rect {
                position: [scaled_rect_pos_x, scaled_rect_pos_y],
                color: line[i].background_color,
                size: [width_bound * quantity, unscaled_sugar_height],
            });

            match &line[i].cursor {
                FragmentCursor::Block(cursor_color) => {
                    self.rects.push(Rect {
                        position: [scaled_rect_pos_x, scaled_rect_pos_y],
                        color: *cursor_color,
                        size: [width_bound * quantity, unscaled_sugar_height],
                    });
                }
                FragmentCursor::Caret(cursor_color) => {
                    self.rects.push(Rect {
                        position: [scaled_rect_pos_x, scaled_rect_pos_y],
                        color: *cursor_color,
                        size: [(width_bound * 0.02) * quantity, unscaled_sugar_height],
                    });
                }
                FragmentCursor::Underline(cursor_color) => {
                    let dec_pos_y = (scaled_rect_pos_y) + tree.layout.font_size - 2.5;
                    self.rects.push(Rect {
                        position: [scaled_rect_pos_x, dec_pos_y],
                        color: *cursor_color,
                        size: [(width_bound * 0.1) * quantity, unscaled_sugar_height],
                    });
                }
                FragmentCursor::Disabled => {}
            }

            match &line[i].decoration {
               &FragmentDecoration::Underline => {
                    let dec_pos_y = (scaled_rect_pos_y) + tree.layout.font_size - 1.;
                    self.rects.push(Rect {
                        position: [scaled_rect_pos_x, dec_pos_y],
                        color: line[i].foreground_color,
                        size: [(width_bound * quantity), unscaled_sugar_height * 0.025],
                    });
                }
               &FragmentDecoration::Strikethrough => {
                    let dec_pos_y = (scaled_rect_pos_y) + tree.layout.font_size / 2.0;
                    self.rects.push(Rect {
                        position: [scaled_rect_pos_x, dec_pos_y],
                        color: line[i].foreground_color,
                        size: [(width_bound * quantity), unscaled_sugar_height * 0.025],
                    });
                }
                &FragmentDecoration::Disabled => {}
            }

            // if let Some(sugar_media) = &line[i].media {
            //     if let Some(rect) = self.graphic_rects.get_mut(&sugar_media.id) {
            //         rect.columns += 1.0;
            //         rect.end_row = self.current_row.into();
            //     } else {
            //         // println!("miss {:?}", sugar_media.id);
            //         self.graphic_rects.insert(
            //             sugar_media.id,
            //             GraphicRect {
            //                 id: sugar_media.id,
            //                 height: sugar_media.height,
            //                 width: sugar_media.width,
            //                 pos_x: scaled_rect_pos_x,
            //                 pos_y: scaled_rect_pos_y,
            //                 columns: 1.0,
            //                 start_row: 1.0,
            //                 end_row: 1.0,
            //             },
            //         );
            //     }
            // }

            x += add_pos_x * quantity;
        }

        self.current_row += 1;
        self.text_y += tree.layout.dimensions.height * tree.layout.line_height;
    }

    #[inline]
    pub fn create_section_from_text(
        &mut self,
        text: &Text,
        tree: &SugarTree,
    ) -> &OwnedSection {
        let font_id = FontId(text.font_id);

        let owned_text = crate::components::text::OwnedText {
            text: text.content.to_owned(),
            scale: PxScale::from(text.font_size * tree.layout.dimensions.scale),
            font_id,
            extra: crate::components::text::Extra {
                color: text.color,
                z: 0.0,
            },
        };

        let layout = if text.single_line {
            crate::components::text::glyph::Layout::default_single_line()
                .v_align(crate::components::text::glyph::VerticalAlign::Center)
                .h_align(crate::components::text::glyph::HorizontalAlign::Left)
        } else {
            crate::components::text::glyph::Layout::default()
                .v_align(crate::components::text::glyph::VerticalAlign::Center)
                .h_align(crate::components::text::glyph::HorizontalAlign::Left)
        };

        let section = crate::components::text::OwnedSection {
            screen_position: (
                text.position.0 * tree.layout.dimensions.scale,
                text.position.1 * tree.layout.dimensions.scale,
            ),
            bounds: (tree.layout.width, tree.layout.height),
            text: vec![owned_text],
            layout,
        };

        self.text_sections.push(section);

        &self.text_sections[self.text_sections.len() - 1]
    }
}
