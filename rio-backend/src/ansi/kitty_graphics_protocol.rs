use sugarloaf::{GraphicData, GraphicId, ResizeCommand, ResizeParameter};

use std::io::{self, Result};
use std::str;
use tracing;

use base64::engine::general_purpose::STANDARD as Base64;
use base64::Engine;

use image_rs::{imageops, GenericImage, GenericImageView, ImageBuffer, Pixel, RgbaImage};

use core::fmt::{Display, Formatter};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum KittyGraphicsError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Image error: {0}")]
    Image(String),
    #[error("Parse error: {0}")]
    Parse(String),
}

fn get<'a>(keys: &BTreeMap<&str, &'a str>, k: &str) -> Option<&'a str> {
    keys.get(k).map(|&s| s)
}

fn geti<T: core::str::FromStr>(keys: &BTreeMap<&str, &str>, k: &str) -> Option<T> {
    get(keys, k).and_then(|s| s.parse().ok())
}

fn set<T: ToString>(
    keys: &mut BTreeMap<&'static str, String>,
    k: &'static str,
    v: &Option<T>,
) {
    if let Some(v) = v {
        keys.insert(k, v.to_string());
    }
}

#[derive(Clone, PartialEq, Eq)]
pub enum KittyImageData {
    /// The data bytes, baes64-encoded fragments.
    /// t='d'
    Direct(String),
    DirectBin(Vec<u8>),
    /// The path to a file containing the data.
    /// t='f'
    File {
        path: String,
        /// the amount of data to read.
        /// S=...
        data_size: Option<u32>,
        /// The offset at which to read.
        /// O=...
        data_offset: Option<u32>,
    },
    /// The path to a temporary file containing the data.
    /// If the path is in a known temporary location,
    /// it should be removed once the data has been read
    /// t='t'
    TemporaryFile {
        path: String,
        /// the amount of data to read.
        /// S=...
        data_size: Option<u32>,
        /// The offset at which to read.
        /// O=...
        data_offset: Option<u32>,
    },

    /// The name of a shared memory object.
    /// Can be opened via shm_open() and then should be removed
    /// via shm_unlink().
    /// On Windows, OpenFileMapping(), MapViewOfFile(), UnmapViewOfFile()
    /// and CloseHandle() are used to access and release the data.
    /// t='s'
    SharedMem {
        name: String,
        /// the amount of data to read.
        /// S=...
        data_size: Option<u32>,
        /// The offset at which to read.
        /// O=...
        data_offset: Option<u32>,
    },
}

impl core::fmt::Debug for KittyImageData {
    fn fmt(&self, fmt: &mut Formatter) -> core::fmt::Result {
        match self {
            Self::Direct(data) => write!(fmt, "Direct({} bytes of data)", data.len()),
            Self::DirectBin(data) => {
                write!(fmt, "DirectBin({} bytes of data)", data.len())
            }
            Self::File {
                path,
                data_offset,
                data_size,
            } => fmt
                .debug_struct("File")
                .field("path", &path)
                .field("data_offset", &data_offset)
                .field("data_size", data_size)
                .finish(),
            Self::TemporaryFile {
                path,
                data_offset,
                data_size,
            } => fmt
                .debug_struct("TemporaryFile")
                .field("path", &path)
                .field("data_offset", &data_offset)
                .field("data_size", data_size)
                .finish(),
            Self::SharedMem {
                name,
                data_offset,
                data_size,
            } => fmt
                .debug_struct("SharedMem")
                .field("name", &name)
                .field("data_offset", &data_offset)
                .field("data_size", data_size)
                .finish(),
        }
    }
}

impl KittyImageData {
    fn from_keys(keys: &BTreeMap<&str, &str>, payload: &[u8]) -> Option<Self> {
        let t = get(keys, "t").unwrap_or("d");

        match t {
            "d" => Some(Self::Direct(String::from_utf8(payload.to_vec()).ok()?)),
            "f" => Some(Self::File {
                path: String::from_utf8(Base64.decode(payload.to_vec()).ok()?).ok()?,
                data_size: geti(keys, "S"),
                data_offset: geti(keys, "O"),
            }),
            "t" => Some(Self::TemporaryFile {
                path: String::from_utf8(Base64.decode(payload.to_vec()).ok()?).ok()?,
                data_size: geti(keys, "S"),
                data_offset: geti(keys, "O"),
            }),
            "s" => Some(Self::SharedMem {
                name: String::from_utf8(Base64.decode(payload.to_vec()).ok()?).ok()?,
                data_size: geti(keys, "S"),
                data_offset: geti(keys, "O"),
            }),
            _ => None,
        }
    }

    fn to_keys(&self, keys: &mut BTreeMap<&'static str, String>) {
        match self {
            Self::Direct(d) => {
                keys.insert("payload", d.to_string());
            }
            Self::DirectBin(d) => {
                keys.insert("payload", Base64.encode(d));
            }
            Self::File {
                path,
                data_offset,
                data_size,
            } => {
                keys.insert("t", "f".to_string());
                keys.insert("payload", Base64.encode(&path));
                set(keys, "S", data_size);
                set(keys, "O", data_offset);
            }
            Self::TemporaryFile {
                path,
                data_offset,
                data_size,
            } => {
                keys.insert("t", "t".to_string());
                keys.insert("payload", Base64.encode(&path));
                set(keys, "S", data_size);
                set(keys, "O", data_offset);
            }
            Self::SharedMem {
                name,
                data_offset,
                data_size,
            } => {
                keys.insert("t", "s".to_string());
                keys.insert("payload", Base64.encode(&name));
                set(keys, "S", data_size);
                set(keys, "O", data_offset);
            }
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum KittyImageVerbosity {
    Verbose,
    OnlyErrors,
    Quiet,
}

impl KittyImageVerbosity {
    fn from_keys(keys: &BTreeMap<&str, &str>) -> Option<Self> {
        match get(keys, "q") {
            None | Some("0") => Some(Self::Verbose),
            Some("1") => Some(Self::OnlyErrors),
            Some("2") => Some(Self::Quiet),
            _ => None,
        }
    }

    fn to_keys(&self, keys: &mut BTreeMap<&'static str, String>) {
        match self {
            Self::Verbose => {}
            Self::OnlyErrors => {
                keys.insert("q", "1".to_string());
            }
            Self::Quiet => {
                keys.insert("q", "2".to_string());
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KittyImageFormat {
    /// f=24
    Rgb,
    /// f=32
    Rgba,
    /// f=100
    Png,
}

impl KittyImageFormat {
    fn from_keys(keys: &BTreeMap<&str, &str>) -> Option<Option<Self>> {
        match get(keys, "f") {
            None => Some(None),
            Some("32") => Some(Some(Self::Rgba)),
            Some("24") => Some(Some(Self::Rgb)),
            Some("100") => Some(Some(Self::Png)),
            _ => None,
        }
    }

    fn to_keys(&self, keys: &mut BTreeMap<&'static str, String>) {
        match self {
            Self::Rgb => keys.insert("f", "24".to_string()),
            Self::Rgba => keys.insert("f", "32".to_string()),
            Self::Png => keys.insert("f", "100".to_string()),
        };
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KittyImageCompression {
    None,
    /// o='z'
    Deflate,
}

impl KittyImageCompression {
    fn from_keys(keys: &BTreeMap<&str, &str>) -> Option<Self> {
        match get(keys, "o") {
            None => Some(Self::None),
            Some("z") => Some(Self::Deflate),
            _ => None,
        }
    }

    fn to_keys(&self, keys: &mut BTreeMap<&'static str, String>) {
        match self {
            Self::None => {}
            Self::Deflate => {
                keys.insert("o", "z".to_string());
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KittyImageTransmit {
    /// f=...
    pub format: Option<KittyImageFormat>,
    /// combination of t=... and d=...
    pub data: KittyImageData,
    /// s=...
    pub width: Option<u32>,
    /// v=...
    pub height: Option<u32>,
    /// The image id.
    /// i=...
    pub image_id: Option<u32>,
    /// The image number
    /// I=...
    pub image_number: Option<u32>,
    /// o=...
    pub compression: KittyImageCompression,

    /// m=0 or m=1
    pub more_data_follows: bool,
}

impl KittyImageTransmit {
    fn from_keys(keys: &BTreeMap<&str, &str>, payload: &[u8]) -> Option<Self> {
        Some(Self {
            format: KittyImageFormat::from_keys(keys)?,
            data: KittyImageData::from_keys(keys, payload)?,
            compression: KittyImageCompression::from_keys(keys)?,
            width: geti(keys, "s"),
            height: geti(keys, "v"),
            image_id: geti(keys, "i"),
            image_number: geti(keys, "I"),
            more_data_follows: match get(keys, "m") {
                None | Some("0") => false,
                Some("1") => true,
                _ => return None,
            },
        })
    }

    fn to_keys(&self, keys: &mut BTreeMap<&'static str, String>) {
        if let Some(f) = &self.format {
            f.to_keys(keys);
        }

        set(keys, "s", &self.width);
        set(keys, "v", &self.height);
        set(keys, "i", &self.image_id);
        set(keys, "I", &self.image_number);
        if self.more_data_follows {
            keys.insert("m", "1".to_string());
        }

        self.compression.to_keys(keys);
        self.data.to_keys(keys);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KittyImagePlacement {
    /// source rectangle bounds.
    /// Default is whole image.
    /// x=...
    pub x: Option<u32>,
    pub y: Option<u32>,
    pub w: Option<u32>,
    pub h: Option<u32>,
    /// Place the image at an offset from the cell.
    /// X,Y must be <= cell metrics
    /// X=...
    pub x_offset: Option<u32>,
    /// Y=...
    pub y_offset: Option<u32>,
    /// Scale so that the image fits within this number of columns
    /// c=...
    pub columns: Option<u32>,
    /// Scale so that the image fits within this number of rows
    /// r=...
    pub rows: Option<u32>,
    /// By default, cursor will move to after the bottom right
    /// cell of the image placement.  do_not_move_cursor cursor
    /// set to true prevents that.
    /// C=0, C=1
    pub do_not_move_cursor: bool,
    /// Give an explicit placement id to this placement.
    /// p=...
    pub placement_id: Option<u32>,
    /// z=...
    pub z_index: Option<i32>,
}

impl KittyImagePlacement {
    fn from_keys(keys: &BTreeMap<&str, &str>) -> Option<Self> {
        Some(Self {
            x: geti(keys, "x"),
            y: geti(keys, "y"),
            w: geti(keys, "w"),
            h: geti(keys, "h"),
            x_offset: geti(keys, "X"),
            y_offset: geti(keys, "Y"),
            columns: geti(keys, "c"),
            rows: geti(keys, "r"),
            placement_id: geti(keys, "p"),
            do_not_move_cursor: match get(keys, "C") {
                None | Some("0") => false,
                Some("1") => true,
                _ => return None,
            },
            z_index: geti(keys, "z"),
        })
    }

    fn to_keys(&self, keys: &mut BTreeMap<&'static str, String>) {
        set(keys, "x", &self.x);
        set(keys, "y", &self.y);
        set(keys, "w", &self.w);
        set(keys, "h", &self.h);
        set(keys, "X", &self.x_offset);
        set(keys, "Y", &self.y_offset);
        set(keys, "c", &self.columns);
        set(keys, "r", &self.rows);
        set(keys, "p", &self.placement_id);

        if self.do_not_move_cursor {
            keys.insert("C", "1".to_string());
        }

        set(keys, "z", &self.z_index);
    }
}

/// When the uppercase form is used, the delete: field is set to true
/// which means that the underlying data is also released.  Otherwise,
/// the data is available to be placed again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KittyImageDelete {
    /// d='a' or d='A'.
    /// Delete all placements on visible screen
    All { delete: bool },
    /// d='i' or d='I'
    /// Delete all images with specified image_id.
    /// If placement_id is specified, then both image_id
    /// and placement_id must match
    ByImageId {
        image_id: u32,
        placement_id: Option<u32>,
        delete: bool,
    },
    /// d='n' or d='N'
    /// Delete newest image with specified image number.
    /// If placement_id is specified, then placement_id
    /// must also match.
    ByImageNumber {
        image_number: u32,
        placement_id: Option<u32>,
        delete: bool,
    },

    /// d='c' or d='C'
    /// Delete all placements that intersect with the current
    /// cursor position.
    AtCursorPosition { delete: bool },

    /// d='f' or d='F'
    /// Delete animation frames
    AnimationFrames { delete: bool },

    /// d='p' or d='P'
    /// Delete all placements that intersect the specified
    /// cell x and y coordinates
    DeleteAt { x: u32, y: u32, delete: bool },

    /// d='q' or d='Q'
    /// Delete all placements that intersect the specified
    /// cell x and y coordinates, with the specified z-index
    DeleteAtZ {
        x: u32,
        y: u32,
        z: i32,
        delete: bool,
    },

    /// d='x' or d='X'
    /// Delete all placements that intersect the specified column.
    DeleteColumn { x: u32, delete: bool },

    /// d='y' or d='Y'
    /// Delete all placements that intersect the specified row.
    DeleteRow { y: u32, delete: bool },

    /// d='z' or d='Z'
    /// Delete all placements that have the specified z-index.
    DeleteZ { z: i32, delete: bool },
}

impl KittyImageDelete {
    fn from_keys(keys: &BTreeMap<&str, &str>) -> Option<Self> {
        let d = get(keys, "d").unwrap_or("a");
        if d.len() != 1 {
            return None;
        }
        let d = d.chars().next()?;
        let delete = d.is_ascii_uppercase();
        match d {
            'a' | 'A' => Some(Self::All { delete }),
            'i' | 'I' => Some(Self::ByImageId {
                image_id: geti(keys, "i")?,
                placement_id: geti(keys, "p"),
                delete,
            }),
            'n' | 'N' => Some(Self::ByImageNumber {
                image_number: geti(keys, "I")?,
                placement_id: geti(keys, "p"),
                delete,
            }),
            'c' | 'C' => Some(Self::AtCursorPosition { delete }),
            'f' | 'F' => Some(Self::AnimationFrames { delete }),
            'p' | 'P' => Some(Self::DeleteAt {
                x: geti(keys, "x")?,
                y: geti(keys, "y")?,
                delete,
            }),
            'q' | 'Q' => Some(Self::DeleteAtZ {
                x: geti(keys, "x")?,
                y: geti(keys, "y")?,
                z: geti(keys, "z")?,
                delete,
            }),
            'x' | 'X' => Some(Self::DeleteColumn {
                x: geti(keys, "x")?,
                delete,
            }),
            'y' | 'Y' => Some(Self::DeleteRow {
                y: geti(keys, "y")?,
                delete,
            }),
            'z' | 'Z' => Some(Self::DeleteZ {
                z: geti(keys, "z")?,
                delete,
            }),
            _ => None,
        }
    }

    fn to_keys(&self, keys: &mut BTreeMap<&'static str, String>) {
        fn d(c: char, delete: &bool) -> String {
            if *delete { c.to_ascii_uppercase() } else { c }.to_string()
        }

        match self {
            Self::All { delete } => {
                keys.insert("d", d('a', delete));
            }
            Self::ByImageId {
                image_id,
                placement_id,
                delete,
            } => {
                keys.insert("d", d('i', delete));
                if let Some(p) = placement_id {
                    keys.insert("p", p.to_string());
                }
                keys.insert("i", image_id.to_string());
            }
            Self::ByImageNumber {
                image_number,
                placement_id,
                delete,
            } => {
                keys.insert("d", d('n', delete));
                if let Some(p) = placement_id {
                    keys.insert("p", p.to_string());
                }
                keys.insert("I", image_number.to_string());
            }
            Self::AtCursorPosition { delete } => {
                keys.insert("d", d('c', delete));
            }
            Self::AnimationFrames { delete } => {
                keys.insert("d", d('f', delete));
            }
            Self::DeleteAt { x, y, delete } => {
                keys.insert("d", d('p', delete));
                keys.insert("x", x.to_string());
                keys.insert("y", y.to_string());
            }
            Self::DeleteAtZ { x, y, z, delete } => {
                keys.insert("d", d('p', delete));
                keys.insert("x", x.to_string());
                keys.insert("y", y.to_string());
                keys.insert("z", z.to_string());
            }
            Self::DeleteColumn { x, delete } => {
                keys.insert("d", d('x', delete));
                keys.insert("x", x.to_string());
            }
            Self::DeleteRow { y, delete } => {
                keys.insert("d", d('y', delete));
                keys.insert("y", y.to_string());
            }
            Self::DeleteZ { z, delete } => {
                keys.insert("d", d('z', delete));
                keys.insert("z", z.to_string());
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KittyFrameCompositionMode {
    AlphaBlending,
    Overwrite,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KittyImageFrameCompose {
    /// i=...
    pub image_id: Option<u32>,
    /// I=...
    pub image_number: Option<u32>,

    /// 1-based number of the frame which should be the base
    /// data for the new frame being created.
    /// If omitted, use background_pixel to specify color.
    /// c=...
    pub target_frame: Option<u32>,

    /// 1-based number of the frame which should be edited.
    /// If omitted, a new frame is created.
    /// r=...
    pub source_frame: Option<u32>,

    /// Left edge in pixels to update
    /// x=...
    pub x: Option<u32>,
    /// Top edge in pixels to update
    /// y=...
    pub y: Option<u32>,

    /// Width (in pixels) of the source and destination rectangles.
    /// By default the full width is used.
    /// w=...
    pub w: Option<u32>,

    /// Height (in pixels) of the source and destination rectangles.
    /// By default the full height is used.
    /// h=...
    pub h: Option<u32>,

    /// Left edge in pixels of the source rectangle
    /// X=...
    pub src_x: Option<u32>,
    /// Top edge in pixels of the source rectangle
    /// Y=...
    pub src_y: Option<u32>,

    /// Composition mode.
    /// Default is AlphaBlending
    /// C=...
    pub composition_mode: KittyFrameCompositionMode,
}

impl KittyImageFrameCompose {
    fn from_keys(keys: &BTreeMap<&str, &str>) -> Option<Self> {
        Some(Self {
            image_id: geti(keys, "i"),
            image_number: geti(keys, "I"),
            x: geti(keys, "x"),
            y: geti(keys, "y"),
            src_x: geti(keys, "X"),
            src_y: geti(keys, "Y"),
            w: geti(keys, "w"),
            h: geti(keys, "h"),
            target_frame: match geti(keys, "c") {
                None | Some(0) => None,
                n => n,
            },
            source_frame: match geti(keys, "r") {
                None | Some(0) => None,
                n => n,
            },
            composition_mode: match geti(keys, "C") {
                None | Some(0) => KittyFrameCompositionMode::AlphaBlending,
                Some(1) => KittyFrameCompositionMode::Overwrite,
                _ => return None,
            },
        })
    }

    fn to_keys(&self, keys: &mut BTreeMap<&'static str, String>) {
        set(keys, "i", &self.image_id);
        set(keys, "I", &self.image_number);
        set(keys, "w", &self.w);
        set(keys, "h", &self.h);
        set(keys, "x", &self.x);
        set(keys, "y", &self.y);
        set(keys, "X", &self.src_x);
        set(keys, "Y", &self.src_y);
        set(keys, "c", &self.target_frame);
        set(keys, "r", &self.source_frame);
        match &self.composition_mode {
            KittyFrameCompositionMode::AlphaBlending => {}
            KittyFrameCompositionMode::Overwrite => {
                keys.insert("C", "1".to_string());
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KittyImageFrame {
    /// Left edge in pixels to update
    pub x: Option<u32>,
    /// Top edge in pixels to update
    pub y: Option<u32>,

    /// 1-based number of the frame which should be the base
    /// data for the new frame being created.
    /// If omitted, use background_pixel to specify color.
    /// c=...
    pub base_frame: Option<u32>,

    /// 1-based number of the frame which should be edited.
    /// If omitted, a new frame is created.
    /// r=...
    pub frame_number: Option<u32>,

    /// Gap in milliseconds of this frame from the next one.
    /// Zero or omitted values are interpreted as 40ms.
    /// z=...
    pub duration_ms: Option<u32>,

    /// Composition mode.
    /// Default is AlphaBlending
    /// X=...
    pub composition_mode: KittyFrameCompositionMode,

    /// Background color for pixels not specified in the frame data.
    /// If omitted, use a black, fully-transparent pixel (0)
    /// Y=...
    pub background_pixel: Option<u32>,
}

impl KittyImageFrame {
    fn from_keys(keys: &BTreeMap<&str, &str>) -> Option<Self> {
        Some(Self {
            x: geti(keys, "x"),
            y: geti(keys, "y"),
            base_frame: match geti(keys, "c") {
                None | Some(0) => None,
                n => n,
            },
            frame_number: match geti(keys, "r") {
                None | Some(0) => None,
                n => n,
            },
            duration_ms: match geti(keys, "Z") {
                None | Some(0) => None,
                n => n,
            },
            composition_mode: match geti(keys, "X") {
                None | Some(0) => KittyFrameCompositionMode::AlphaBlending,
                Some(1) => KittyFrameCompositionMode::Overwrite,
                _ => return None,
            },
            background_pixel: geti(keys, "Y"),
        })
    }

    fn to_keys(&self, keys: &mut BTreeMap<&'static str, String>) {
        set(keys, "x", &self.x);
        set(keys, "y", &self.y);
        set(keys, "c", &self.base_frame);
        set(keys, "r", &self.frame_number);
        set(keys, "Z", &self.duration_ms);
        match &self.composition_mode {
            KittyFrameCompositionMode::AlphaBlending => {}
            KittyFrameCompositionMode::Overwrite => {
                keys.insert("X", "1".to_string());
            }
        }
        set(keys, "Y", &self.background_pixel);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KittyImage {
    /// a='t'
    TransmitData {
        transmit: KittyImageTransmit,
        verbosity: KittyImageVerbosity,
    },
    /// a='T'
    TransmitDataAndDisplay {
        transmit: KittyImageTransmit,
        placement: KittyImagePlacement,
        verbosity: KittyImageVerbosity,
    },
    /// a='p'
    Display {
        image_id: Option<u32>,
        image_number: Option<u32>,
        placement: KittyImagePlacement,
        verbosity: KittyImageVerbosity,
    },
    /// a='d'
    Delete {
        what: KittyImageDelete,
        verbosity: KittyImageVerbosity,
    },
    /// a='q'
    Query { transmit: KittyImageTransmit },
    /// a='f'
    TransmitFrame {
        transmit: KittyImageTransmit,
        frame: KittyImageFrame,
        verbosity: KittyImageVerbosity,
    },
    /// a='c'
    ComposeFrame {
        frame: KittyImageFrameCompose,
        verbosity: KittyImageVerbosity,
    },
}

impl KittyImage {
    pub fn verbosity(&self) -> KittyImageVerbosity {
        match self {
            Self::TransmitData { verbosity, .. } => *verbosity,
            Self::Query { .. } => KittyImageVerbosity::Verbose,
            Self::TransmitDataAndDisplay { verbosity, .. } => *verbosity,
            Self::Display { verbosity, .. } => *verbosity,
            Self::Delete { verbosity, .. } => *verbosity,
            Self::TransmitFrame { verbosity, .. } => *verbosity,
            Self::ComposeFrame { verbosity, .. } => *verbosity,
        }
    }

    pub fn parse_apc(data: &[u8]) -> Option<Self> {
        if data.is_empty() || data[0] != b'G' {
            return None;
        }
        let mut keys_payload_iter = data[1..].splitn(2, |&d| d == b';');
        let keys = keys_payload_iter.next()?;
        let key_string = core::str::from_utf8(keys).ok()?;
        let mut keys: BTreeMap<&str, &str> = BTreeMap::new();
        for k_v in key_string.split(',') {
            let mut k_v = k_v.splitn(2, '=');
            let k = k_v.next()?;
            let v = k_v.next()?;
            keys.insert(k, v);
        }

        let payload = keys_payload_iter.next().unwrap_or(b"");
        let action = get(&keys, "a").unwrap_or("t");
        let verbosity = KittyImageVerbosity::from_keys(&keys)?;
        match action {
            "t" => Some(Self::TransmitData {
                transmit: KittyImageTransmit::from_keys(&keys, payload)?,
                verbosity,
            }),
            "q" => Some(Self::Query {
                transmit: KittyImageTransmit::from_keys(&keys, payload)?,
            }),
            "T" => Some(Self::TransmitDataAndDisplay {
                transmit: KittyImageTransmit::from_keys(&keys, payload)?,
                placement: KittyImagePlacement::from_keys(&keys)?,
                verbosity,
            }),
            "p" => Some(Self::Display {
                placement: KittyImagePlacement::from_keys(&keys)?,
                image_id: geti(&keys, "i"),
                image_number: geti(&keys, "I"),
                verbosity,
            }),
            "d" => Some(Self::Delete {
                what: KittyImageDelete::from_keys(&keys)?,
                verbosity,
            }),
            "f" => Some(Self::TransmitFrame {
                transmit: KittyImageTransmit::from_keys(&keys, payload)?,
                frame: KittyImageFrame::from_keys(&keys)?,
                verbosity,
            }),
            "c" => Some(Self::ComposeFrame {
                frame: KittyImageFrameCompose::from_keys(&keys)?,
                verbosity,
            }),
            _ => None,
        }
    }

    fn to_keys(&self, keys: &mut BTreeMap<&'static str, String>) {
        match self {
            Self::TransmitData {
                transmit,
                verbosity,
            } => {
                // Implied: keys.insert("a", "t".to_string());
                verbosity.to_keys(keys);
                transmit.to_keys(keys);
            }
            Self::Query { transmit } => {
                keys.insert("a", "q".to_string());
                transmit.to_keys(keys);
            }
            Self::TransmitDataAndDisplay {
                transmit,
                verbosity,
                placement,
            } => {
                keys.insert("a", "Q".to_string());
                verbosity.to_keys(keys);
                placement.to_keys(keys);
                transmit.to_keys(keys);
            }
            Self::Display {
                image_id,
                image_number,
                placement,
                verbosity,
            } => {
                keys.insert("a", "p".to_string());
                verbosity.to_keys(keys);
                placement.to_keys(keys);
                if let Some(image_id) = image_id {
                    keys.insert("i", image_id.to_string());
                }
                if let Some(image_number) = image_number {
                    keys.insert("I", image_number.to_string());
                }
            }
            Self::Delete { what, verbosity } => {
                keys.insert("a", "d".to_string());
                verbosity.to_keys(keys);
                what.to_keys(keys);
            }
            Self::TransmitFrame {
                transmit,
                verbosity,
                frame,
            } => {
                keys.insert("a", "f".to_string());
                transmit.to_keys(keys);
                frame.to_keys(keys);
                verbosity.to_keys(keys);
            }
            Self::ComposeFrame { frame, verbosity } => {
                keys.insert("a", "c".to_string());
                frame.to_keys(keys);
                verbosity.to_keys(keys);
            }
        }
    }
}

impl Display for KittyImage {
    fn fmt(&self, f: &mut Formatter) -> std::fmt::Result {
        write!(f, "\x1b_G")?;
        let mut keys = BTreeMap::new();
        self.to_keys(&mut keys);
        let mut payload = None;
        let mut first = true;
        for (k, v) in keys {
            if k == "payload" {
                payload = Some(v);
            } else {
                if first {
                    first = false;
                } else {
                    write!(f, ",")?;
                }

                write!(f, "{}={}", k, v)?;
            }
        }

        if let Some(p) = payload {
            write!(f, ";{}", p)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn kitty_payload() {
        assert_eq!(
            KittyImage::parse_apc("Gf=24,s=10,v=20;aGVsbG8=".as_bytes()).unwrap(),
            KittyImage::TransmitData {
                transmit: KittyImageTransmit {
                    format: Some(KittyImageFormat::Rgb),
                    data: KittyImageData::Direct("aGVsbG8=".to_string()),
                    width: Some(10),
                    height: Some(20),
                    image_id: None,
                    image_number: None,
                    compression: KittyImageCompression::None,
                    more_data_follows: false,
                },
                verbosity: KittyImageVerbosity::Verbose,
            }
        );

        assert_eq!(
            KittyImage::parse_apc("Ga=d,q=2".as_bytes()).unwrap(),
            KittyImage::Delete {
                what: KittyImageDelete::All { delete: false },
                verbosity: KittyImageVerbosity::Quiet
            }
        );

        assert_eq!(
            KittyImage::parse_apc(
                "Ga=f,x=119,y=384,s=17,v=32,i=7257421,X=1,r=1,q=2;AAAA=".as_bytes()
            )
            .unwrap(),
            KittyImage::TransmitFrame {
                transmit: KittyImageTransmit {
                    format: None,
                    data: KittyImageData::Direct("AAAA=".to_string()),
                    width: Some(17),
                    height: Some(32),
                    image_id: Some(7257421),
                    image_number: None,
                    compression: KittyImageCompression::None,
                    more_data_follows: false,
                },
                verbosity: KittyImageVerbosity::Quiet,
                frame: KittyImageFrame {
                    x: Some(119),
                    y: Some(384),
                    base_frame: None,
                    frame_number: Some(1),
                    composition_mode: KittyFrameCompositionMode::Overwrite,
                    background_pixel: None,
                    duration_ms: None,
                },
            }
        );
    }

    #[test]
    fn test_handle_query_bell() {
        let params = &[b"Gi=1,a=q,q=1".as_slice()];
        let resp = handle_query(params, "\x07").expect("expected response");
        assert_eq!(resp, "\x1b_Gi=1,a=v,q=1;OK\x07");
    }

    #[test]
    fn test_handle_query_st() {
        let params = &[b"Gi=1,a=q,q=1".as_slice()];
        let resp = handle_query(params, "\x1b\\").expect("expected response");
        assert_eq!(resp, "\x1b_Gi=1,a=v,q=1\x1b\\");
    }

    #[test]
    fn test_handle_query_without_g_prefix() {
        let params = &[b"i=1,a=q,q=1".as_slice()];
        let resp = handle_query(params, "\x07").expect("expected response");
        assert_eq!(resp, "\x1b_Gi=1,a=v,q=1;OK\x07");
    }

    #[test]
    fn test_handle_query_no_action() {
        let params = &[b"i=1,q=1".as_slice()];
        let resp = handle_query(params, "\x07");
        assert_eq!(resp, None);
    }

    #[test]
    fn test_handle_query_empty_params() {
        let params: &[&[u8]] = &[];
        let resp = handle_query(params, "\x07");
        assert_eq!(resp, None);
    }

    #[test]
    fn test_chunk_accumulation_single_chunk() {
        // Test single chunk (m=0) - should decode immediately
        // This is a minimal valid 1x1 red PNG encoded as base64
        let png_base64 = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8DwHwAFBQIAX8jx0gAAAABJRU5ErkJggg==";
        let apc_data = format!("Gf=100,i=1,m=0;{}", png_base64);

        let mut state = KittyImageState::default();
        let params = &[apc_data.as_bytes()];
        let result = parse(params, &mut state);

        assert!(
            result.is_some(),
            "Single chunk with m=0 should produce GraphicData"
        );
        let graphic = result.unwrap();
        assert!(graphic.width > 0, "Decoded image should have width > 0");
        assert!(graphic.height > 0, "Decoded image should have height > 0");
    }

    #[test]
    fn test_chunk_accumulation_multi_chunk() {
        // Test multi-chunk assembly: split base64 PNG across two chunks
        // First chunk has m=1 (more follows), second has m=0 (final)
        let png_base64 = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8DwHwAFBQIAX8jx0gAAAABJRU5ErkJggg==";

        // Split the base64 roughly in half
        let split_point = png_base64.len() / 2;
        let chunk1 = &png_base64[..split_point];
        let chunk2 = &png_base64[split_point..];

        let apc_chunk1 = format!("Gf=100,i=42,m=1;{}", chunk1);
        let apc_chunk2 = format!("Gf=100,i=42,m=0;{}", chunk2);

        let mut state = KittyImageState::default();

        // First chunk: should return None and accumulate
        let params1 = &[apc_chunk1.as_bytes()];
        let result1 = parse(params1, &mut state);
        assert!(result1.is_none(), "First chunk with m=1 should return None");
        assert!(
            state.chunk_accumulators.contains_key(&42),
            "State should have accumulated data for image_id=42"
        );

        // Second chunk: should complete and return GraphicData
        let params2 = &[apc_chunk2.as_bytes()];
        let result2 = parse(params2, &mut state);
        assert!(
            result2.is_some(),
            "Final chunk with m=0 should produce GraphicData"
        );

        let graphic = result2.unwrap();
        assert!(graphic.width > 0, "Decoded image should have width > 0");
        assert!(graphic.height > 0, "Decoded image should have height > 0");

        // State should be cleaned up
        assert!(
            !state.chunk_accumulators.contains_key(&42),
            "Accumulator should be removed after final chunk"
        );
    }

    #[test]
    fn test_chunk_accumulation_multiple_images() {
        // Test that chunks for different image IDs are tracked separately
        let png_base64 = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8DwHwAFBQIAX8jx0gAAAABJRU5ErkJggg==";
        let split_point = png_base64.len() / 2;
        let chunk1 = &png_base64[..split_point];
        let chunk2 = &png_base64[split_point..];

        let mut state = KittyImageState::default();

        // Start two different images
        let apc_img1_chunk1 = format!("Gf=100,i=100,m=1;{}", chunk1);
        let apc_img2_chunk1 = format!("Gf=100,i=200,m=1;{}", chunk1);

        parse(&[apc_img1_chunk1.as_bytes()], &mut state);
        parse(&[apc_img2_chunk1.as_bytes()], &mut state);

        assert!(state.chunk_accumulators.contains_key(&100));
        assert!(state.chunk_accumulators.contains_key(&200));

        // Complete image 200 first
        let apc_img2_chunk2 = format!("Gf=100,i=200,m=0;{}", chunk2);
        let result = parse(&[apc_img2_chunk2.as_bytes()], &mut state);
        assert!(result.is_some(), "Image 200 should complete");
        assert!(
            !state.chunk_accumulators.contains_key(&200),
            "Image 200 accumulator should be removed"
        );
        assert!(
            state.chunk_accumulators.contains_key(&100),
            "Image 100 should still be accumulating"
        );

        // Complete image 100
        let apc_img1_chunk2 = format!("Gf=100,i=100,m=0;{}", chunk2);
        let result = parse(&[apc_img1_chunk2.as_bytes()], &mut state);
        assert!(result.is_some(), "Image 100 should complete");
        assert!(
            !state.chunk_accumulators.contains_key(&100),
            "Image 100 accumulator should be removed"
        );
    }
}

/// Handle Kitty graphics protocol queries
/// Returns a response string if this was a query, None otherwise
/// The terminator indicates whether the original APC sequence ended with BEL (0x07) or ST (ESC \)
pub fn handle_query(params: &[&[u8]], terminator: &str) -> Option<String> {
    tracing::debug!(
        "handle_query called with {} params, terminator: {}",
        params.len(),
        terminator
    );

    if params.is_empty() {
        tracing::debug!("No params in handle_query");
        return None;
    }

    // Check if this looks like a Kitty graphics query by examining all parameters
    // kitten icat sends queries like: a=q,f=24,s=1,v=1,S=3,i=1;payload
    // The i= parameter is the image ID used for the response

    let mut has_query_action = false;
    let mut image_id: u32 = 0;

    for (idx, param) in params.iter().enumerate() {
        let param_str = std::str::from_utf8(param).ok()?;
        tracing::debug!("handle_query param[{}]: {}", idx, param_str);

        // Split off any payload after ';'
        let control_part = param_str.split(';').next().unwrap_or(param_str);

        // Check for query action and extract image_id
        for part in control_part.split(',') {
            if part == "a=q" {
                has_query_action = true;
                tracing::debug!("Found a=q in param[{}]", idx);
            }
            if let Some(i) = part.strip_prefix("i=") {
                if let Ok(id) = i.parse::<u32>() {
                    image_id = id;
                    tracing::debug!("Found image_id: {}", id);
                }
            }
        }
    }

    if !has_query_action {
        tracing::debug!("No query action found in params - not a graphics query");
        return None;
    }

    tracing::info!(
        "Detected Kitty graphics query: image_id={}",
        image_id
    );

    // Response format: ESC_G i=ID;OK <terminator>
    // The i= value must match what was sent in the query
    let response = format!("\x1b_Gi={};OK{}", image_id, terminator);

    tracing::info!("Query response: {}", response);

    Some(response)
}

#[derive(Debug, Default)]
pub struct KittyImageState {
    accumulator: Vec<KittyImage>,
    max_image_id: u32,
    number_to_id: HashMap<u32, u32>,
    id_to_data: HashMap<u32, Arc<GraphicData>>,
    placements: HashMap<(u32, u32), ()>,

    used_memory: usize,

    pub chunk_accumulators: HashMap<u32, String>,
}

impl KittyImageState {
    fn remove_data_for_id(&mut self, image_id: u32) {
        if let Some(data) = self.id_to_data.remove(&image_id) {
            self.used_memory = self.used_memory.saturating_sub(data.pixels.len());
        }
    }

    fn record_id_to_data(&mut self, image_id: u32, data: Arc<GraphicData>) {
        if image_id != 0 {
            self.remove_data_for_id(image_id);
        }
        self.prune_unreferenced();
        self.used_memory += data.pixels.len();
        self.id_to_data.insert(image_id, data);
    }

    fn prune_unreferenced(&mut self) {
        let budget = 320 * 1024 * 1024; // FIXME: make this configurable
        if self.used_memory > budget {
            let referenced: HashSet<u32> =
                self.placements.keys().map(|(k, _)| *k).collect();
            let target = self.used_memory - budget;
            let mut freed = 0;
            self.id_to_data.retain(|id, data| {
                if referenced.contains(id) || freed > target {
                    true
                } else {
                    freed += data.pixels.len();
                    false
                }
            });

            tracing::info!(
                "using {} RAM for images, pruned {}",
                self.used_memory,
                freed
            );
            self.used_memory = self.used_memory.saturating_sub(freed);
        }
    }
}

/// Make a copy of the source region.
/// Ideally we wouldn't need this, but Rust's mutability rules
/// make it very awkward to mutably reference a frame while
/// an immutable reference exists to a separate frame.
fn clip_view(
    width: u32,
    height: u32,
    data: &mut [u8],
    src_x: Option<u32>,
    src_y: Option<u32>,
    view_width: Option<u32>,
    view_height: Option<u32>,
) -> Result<RgbaImage> {
    let src = ImageBuffer::from_raw(width, height, data).ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidData, "ill formed image")
    })?;

    let src_x = src_x.unwrap_or(0);
    let src_y = src_y.unwrap_or(0);

    let view_width = view_width.unwrap_or(width);
    let view_height = view_height.unwrap_or(height);

    let (view_width, view_height) = image_rs::imageops::overlay_bounds(
        (width, height),
        (view_width, view_height),
        src_x,
        src_y,
    );

    let view = src.view(src_x, src_y, view_width, view_height);

    let mut tmp = RgbaImage::new(view_width, view_height);
    tmp.copy_from(&*view, 0, 0)
        .map_err(|e| io::Error::other(format!("copy source image: {}", e)))?;
    Ok(tmp)
}

fn blit<D, S, P>(
    dest: &mut D,
    src: &S,
    x: u32,
    y: u32,
    mode: KittyFrameCompositionMode,
) -> std::result::Result<(), KittyGraphicsError>
where
    D: GenericImage<Pixel = P>,
    S: GenericImageView<Pixel = P>,
    P: Pixel<Subpixel = u8>,
{
    match mode {
        KittyFrameCompositionMode::Overwrite => {
            imageops::overlay(dest, src, x.into(), y.into());
        }
        KittyFrameCompositionMode::AlphaBlending => {
            let (src_width, src_height) = src.dimensions();
            let dest_width = dest.width();
            let dest_height = dest.height();

            for dy in 0..src_height {
                for dx in 0..src_width {
                    let px = x + dx;
                    let py = y + dy;

                    // Check bounds before accessing
                    if px < dest_width && py < dest_height {
                        let dp = dest.get_pixel(px, py);
                        let sp = src.get_pixel(dx, dy);
                        let mut blended = dp;
                        blended.blend(&sp);
                        dest.put_pixel(px, py, blended);
                        let _ = blended;
                    }
                }
            }
        }
    }
    Ok(())
}

pub fn parse(params: &[&[u8]], state: &mut KittyImageState) -> Option<GraphicData> {
    if params.is_empty() {
        return None;
    }

    // Parse the APC data
    let apc_data = params[0];
    let kitty_image = KittyImage::parse_apc(apc_data).map_or(
        {
            tracing::info!("Failed to parse Kitty APC data");
            None
        },
        Some,
    )?;

    // Handle different Kitty image actions
    match kitty_image {
        KittyImage::TransmitData { transmit, .. }
        | KittyImage::TransmitDataAndDisplay { transmit, .. } => {
            // Process image data transmission
            match transmit.data {
                KittyImageData::Direct(base64_data) => {
                    let image_id = transmit.image_id.unwrap_or(0);

                    if transmit.more_data_follows {
                        tracing::info!(
                            "Received Kitty image chunk for id={}, len={}, more follows",
                            image_id,
                            base64_data.len()
                        );
                        state
                            .chunk_accumulators
                            .entry(image_id)
                            .or_default()
                            .push_str(&base64_data);
                        return None;
                    }

                    let final_data = if let Some(mut accumulated) =
                        state.chunk_accumulators.remove(&image_id)
                    {
                        tracing::info!(
                            "Received final Kitty image chunk for id={}, len={}, total accumulated len={}",
                            image_id,
                            base64_data.len(),
                            accumulated.len() + base64_data.len()
                        );
                        accumulated.push_str(&base64_data);
                        accumulated
                    } else {
                        base64_data
                    };

                    // Decode base64 data
                    let decoded = Base64
                        .decode(final_data.as_bytes())
                        .map_err(|e| {
                            tracing::info!(
                                "Failed to decode base64 (len={}): {:?}",
                                final_data.len(),
                                e
                            );
                            KittyGraphicsError::Parse(format!(
                                "Failed to decode base64: {:?}",
                                e
                            ))
                        })
                        .ok()?;

                    tracing::info!(
                        "Successfully decoded Kitty image base64, size={} bytes",
                        decoded.len()
                    );

                    // Load image from memory
                    let dynamic_image = image_rs::load_from_memory(&decoded)
                        .map_err(|e| {
                            tracing::info!(
                                "Failed to load image from memory (size={}): {:?}",
                                decoded.len(),
                                e
                            );
                            KittyGraphicsError::Image(format!(
                                "Failed to load image from memory: {:?}",
                                e
                            ))
                        })
                        .ok()?;

                    // Convert to GraphicData
                    let mut graphic_data =
                        GraphicData::from_dynamic_image(GraphicId(0), dynamic_image);

                    // Apply resize parameters if specified
                    let resize_width = transmit.width.map(ResizeParameter::Pixels);
                    let resize_height = transmit.height.map(ResizeParameter::Pixels);

                    if resize_width.is_some() || resize_height.is_some() {
                        graphic_data.resize = Some(ResizeCommand {
                            width: resize_width.unwrap_or(ResizeParameter::Auto),
                            height: resize_height.unwrap_or(ResizeParameter::Auto),
                            preserve_aspect_ratio: true,
                        });
                    }

                    Some(graphic_data)
                }
                KittyImageData::SharedMem {
                    name,
                    data_size,
                    data_offset,
                } => {
                    tracing::info!(
                        "Reading Kitty image from shared memory: {}, size={:?}, offset={:?}",
                        name,
                        data_size,
                        data_offset
                    );

                    let shm_path = format!("/dev/shm/{}", name);
                    let image_bytes = match std::fs::read(&shm_path) {
                        Ok(mut bytes) => {
                            let offset = data_offset.unwrap_or(0) as usize;
                            if offset > 0 && offset < bytes.len() {
                                bytes = bytes[offset..].to_vec();
                            }
                            if let Some(size) = data_size {
                                let size = size as usize;
                                if size < bytes.len() {
                                    bytes.truncate(size);
                                }
                            }
                            let _ = std::fs::remove_file(&shm_path);
                            bytes
                        }
                        Err(e) => {
                            tracing::warn!("Failed to read shared memory {}: {:?}", shm_path, e);
                            return None;
                        }
                    };

                    tracing::info!(
                        "Read {} bytes from shared memory, format={:?}, width={:?}, height={:?}",
                        image_bytes.len(),
                        transmit.format,
                        transmit.width,
                        transmit.height
                    );

                    let dynamic_image = match transmit.format {
                        Some(KittyImageFormat::Rgb) => {
                            let width = transmit.width.unwrap_or(0);
                            let height = transmit.height.unwrap_or(0);
                            if width == 0 || height == 0 {
                                tracing::warn!("Raw RGB data requires width and height");
                                return None;
                            }
                            image_rs::RgbImage::from_raw(width, height, image_bytes)
                                .map(image_rs::DynamicImage::ImageRgb8)
                        }
                        Some(KittyImageFormat::Rgba) => {
                            let width = transmit.width.unwrap_or(0);
                            let height = transmit.height.unwrap_or(0);
                            if width == 0 || height == 0 {
                                tracing::warn!("Raw RGBA data requires width and height");
                                return None;
                            }
                            image_rs::RgbaImage::from_raw(width, height, image_bytes)
                                .map(image_rs::DynamicImage::ImageRgba8)
                        }
                        Some(KittyImageFormat::Png) | None => {
                            image_rs::load_from_memory(&image_bytes).ok()
                        }
                    };

                    let dynamic_image = match dynamic_image {
                        Some(img) => img,
                        None => {
                            tracing::warn!("Failed to create image from shared memory data");
                            return None;
                        }
                    };

                    let graphic_data =
                        GraphicData::from_dynamic_image(GraphicId(0), dynamic_image);

                    Some(graphic_data)
                }
                KittyImageData::TemporaryFile {
                    path,
                    data_size,
                    data_offset,
                } => {
                    tracing::info!(
                        "Reading Kitty image from temp file: {}, size={:?}, offset={:?}",
                        path,
                        data_size,
                        data_offset
                    );

                    let image_bytes = match std::fs::read(&path) {
                        Ok(mut bytes) => {
                            let offset = data_offset.unwrap_or(0) as usize;
                            if offset > 0 && offset < bytes.len() {
                                bytes = bytes[offset..].to_vec();
                            }
                            if let Some(size) = data_size {
                                let size = size as usize;
                                if size < bytes.len() {
                                    bytes.truncate(size);
                                }
                            }
                            let _ = std::fs::remove_file(&path);
                            bytes
                        }
                        Err(e) => {
                            tracing::warn!("Failed to read temp file {}: {:?}", path, e);
                            return None;
                        }
                    };

                    tracing::info!("Read {} bytes from temp file", image_bytes.len());

                    let dynamic_image = image_rs::load_from_memory(&image_bytes)
                        .map_err(|e| {
                            tracing::warn!("Failed to load image from temp file: {:?}", e);
                            e
                        })
                        .ok()?;

                    let mut graphic_data =
                        GraphicData::from_dynamic_image(GraphicId(0), dynamic_image);

                    let resize_width = transmit.width.map(ResizeParameter::Pixels);
                    let resize_height = transmit.height.map(ResizeParameter::Pixels);

                    if resize_width.is_some() || resize_height.is_some() {
                        graphic_data.resize = Some(ResizeCommand {
                            width: resize_width.unwrap_or(ResizeParameter::Auto),
                            height: resize_height.unwrap_or(ResizeParameter::Auto),
                            preserve_aspect_ratio: true,
                        });
                    }

                    Some(graphic_data)
                }
                _ => {
                    tracing::warn!("Unsupported Kitty image data type");
                    None
                }
            }
        }
        // Handle display-only commands
        KittyImage::Display { image_id, .. } => {
            tracing::info!("Display image with id: {:?}", image_id);
            None // Display handled separately
        }
        // Handle delete commands
        KittyImage::Delete { what, .. } => {
            tracing::info!("Delete command: {:?}", what);
            None // Delete handled separately
        }
        // Handle other commands
        _ => {
            tracing::info!("Unsupported Kitty image command");
            None
        }
    }
}
