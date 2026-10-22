//! Shared reader data types (split from design/claude-v04/reader_backend.rs `pub mod model`).

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// (book_id, virtual_page_idx, variant). variant 0 = full size, N = max width N px (160 = thumbnail).
pub type Key = (i64, u32, u16);
/// Whitelist: bounds cache variants. 240 added per INTEGRATION_NOTES §3.4 (cover width).
pub const VARIANTS: [u16; 7] = [0, 160, 240, 800, 1200, 1600, 2400];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SourceKind {
    Zip,
    Folder,
    Images,
}
impl SourceKind {
    pub fn from_db(s: &str) -> Option<Self> {
        match s {
            "zip" => Some(Self::Zip),
            "folder" => Some(Self::Folder),
            "images" => Some(Self::Images),
            _ => None,
        }
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Zip => "zip",
            Self::Folder => "folder",
            Self::Images => "images",
        }
    }
}

/// One REAL page as stored in SQLite (dims straight from the file header).
#[derive(Clone, Debug)]
pub struct RealPage {
    pub entry: u32,
    pub name: String,
    pub w: u32,
    pub h: u32,
    pub bytes: u64,
    /// Page-edit overlay only: this page comes from a DIFFERENT source than the
    /// book itself (appended/merged in via the page directory). Absolute file
    /// path read directly by the extractor, bypassing the book's own source.
    /// None for every page produced by a normal scan.
    pub src_path: Option<String>,
}

/// One VIRTUAL page = what JS gets a box for. Tall images are cut into several of these.
#[derive(Clone, Debug)]
pub struct PageMeta {
    pub src_page: u32, // real page number
    pub entry: u32,    // zip entry index (Zip); unused otherwise
    pub name: String,  // zip entry name | path relative to folder | absolute path (Images)
    pub w: u32,
    pub h: u32, // dims of THIS box (h = slice height); 0,0 = unknown
    pub full_h: u32, // height of the whole underlying image
    pub y0: u32,     // slice offset inside the image
    /// See RealPage::src_path (appended/merged pages read from this file).
    pub src_path: Option<String>,
}
impl PageMeta {
    pub fn is_slice(&self) -> bool {
        self.h != 0 && self.h < self.full_h
    }
}

pub struct BookMeta {
    pub id: i64,
    pub kind: SourceKind,
    pub path: PathBuf,
    pub sig: String,
    pub pages: Vec<PageMeta>,
    pub groups: Vec<(u32, u32)>, // real page -> (first, last) virtual idx
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug)]
pub struct SavedPosition {
    pub page: u32,
    pub frac: f64,
}

/// Raw folder-derived chapter start (internal; produced by scan::chapters). Clients get `Episode`.
#[derive(Serialize)]
pub struct Chapter {
    pub page: u32,
    pub title: String,
}

/// How pages are grouped into episodes. A NAVIGATION/DISPLAY layer only: it never touches virtual
/// pages, slicing, cache keys or the extraction queue, so it cannot break the seamless strip.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum ChapterMode {
    #[serde(rename = "none")]
    Off, // continuous: no chapter UI at all
    #[serde(rename = "folder")]
    Folder, // episodes = parent-folder changes (computed from names)
    #[serde(rename = "pages")]
    Pages, // N pages = 1 episode (computed from page count)
}
impl ChapterMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Off => "none",
            Self::Folder => "folder",
            Self::Pages => "pages",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "none" => Some(Self::Off),
            "folder" => Some(Self::Folder),
            "pages" => Some(Self::Pages),
            _ => None,
        }
    }
}

/// index is 1-based; page is the 0-based first REAL page; title is a folder path (folder mode) or "" (pages mode).
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Episode {
    pub index: u32,
    pub page: u32,
    pub len: u32,
    pub title: String,
}
#[derive(Serialize, Deserialize, Clone, Copy, Debug)]
pub struct ChapterSettings {
    pub mode: ChapterMode,
    pub pages: u32,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EpisodeQuery {
    pub book_id: i64,
    pub page: u32,
}
#[derive(Serialize)]
pub struct EpisodeInfo {
    pub index: u32,
    pub total: u32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenedBook {
    pub book_id: i64,
    pub title: String,
    pub sig: String,
    pub real_pages: u32,
    pub dims: Vec<[u32; 2]>, // per VIRTUAL page [w, h]   (0,0 = unknown)
    pub src: Vec<u32>,       // per VIRTUAL page: real page index
    pub chapters: Vec<Episode>, // EFFECTIVE list for this book's mode (empty = show no chapter UI)
    pub folder_chapters: Vec<Episode>, // folder-derived list whatever the mode, so JS can switch modes with no round trip
    pub chapter_mode: ChapterMode, // effective mode for this book (per-book override, else global default)
    pub chapter_pages: u32,        // effective N for "pages" mode
    pub chapter_overridden: bool,  // true = this book has its own row in book_settings ("remember for this book" is on)
    pub resume: Option<SavedPosition>,
}
#[derive(Deserialize)]
pub struct DimFix {
    pub page: u32,
    pub w: u32,
    pub h: u32,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheStats {
    pub bytes: u64,
    pub limit_bytes: u64,
    pub entries: u64,
}
