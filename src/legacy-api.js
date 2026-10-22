// legacy-api.js - central Tauri adapter for the reader + library pages.
// v0.5: every call below is backed by a REAL command (main.rs shelf/folder
// commands, adapters.rs, chapters/bookmarks, and the new library.rs page-edit
// + library-management commands). Nothing here guesses a command name.
const T = window.__TAURI__;
const invoke = (cmd, args = {}) => {
  if (!T?.core?.invoke) return Promise.reject(new Error('当前页面未连接桌面端后端（Tauri）。'));
  return T.core.invoke(cmd, args);
};

export const lib = {
  // reading
  recentBooks: (limit = 12) => invoke('list_recent_books', { limit }),
  recentAdded: (limit = 12) => invoke('list_recent_added', { limit }),
  open: (bookId) => invoke('open_book', { bookId }),
  rescan: (bookId) => invoke('rescan_book', { bookId }),
  relinkBook: (bookId, path) => invoke('relink_book', { bookId, path }),
  revealBook: (bookId) => invoke('reveal_book', { bookId }),

  // page edits (library.rs) - pages are 0-based REAL page indices
  reorderPages: (bookId, mode) => invoke('reorder_pages', { bookId, mode }),
  deletePages: (bookId, pages) => invoke('delete_pages', { bookId, pages }),
  appendPages: (bookId, sources) => invoke('append_pages', { bookId, sources }),
  resetPageEdits: (bookId) => invoke('reset_page_edits', { bookId }),
  pageEditInfo: (bookId) => invoke('page_edit_info', { bookId }),

  // library management
  allBooks: () => invoke('shelf_list'),
  rename: (bookId, name) => invoke('shelf_rename', { bookId, name }),
  remove: (id) => invoke('shelf_remove', { id }),
  moveTo: (id, folderId) => invoke('shelf_move', { id, folder_id: folderId }),
  startOver: (bookId) => invoke('book_reset_progress', { bookId }),
  folders: () => invoke('folder_list'),
  newFolder: (name) => invoke('folder_add', { name }),
  renameFolder: (id, name) => invoke('folder_rename', { id, name }),
  deleteFolder: (id) => invoke('folder_remove', { id }),
  missing: () => invoke('check_books_missing'),
  stats: () => invoke('library_stats'),
  importPaths: (paths, folderId = null) => invoke('import_paths', { paths, folderId }),
  importMerged: (paths, name = null, folderId = null) =>
    invoke('import_merged', { paths, name, folderId }),
};

export { invoke };
