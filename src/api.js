// api.js - central Tauri adapter. Every call is backed by a real command; no new
// commands are introduced here. Connection errors come from the dictionaries;
// shelfGet uses shelf_get. Note: shelf_move declares rename_all = "snake_case" in
// shelf.rs, so moveTo must send folder_id (snake_case) — unlike every other call
// here, which is camelCase.
import { t } from './shell.js';

const T = window.__TAURI__;
const invoke = (cmd, args = {}) => {
  if (!T?.core?.invoke) return Promise.reject(new Error(t('err.noBackend')));
  return T.core.invoke(cmd, args);
};

export const lib = {
  // reading
  recentBooks: (limit = 12) => invoke('list_recent_books', { limit }),
  recentAdded: (limit = 12) => invoke('list_recent_added', { limit }),
  listImages: (path) => invoke('list_images', { path }),   // recursive: every nested subfolder, names relative to `path`
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
  pageEditInfos: (bookIds) => invoke('page_edit_infos', { bookIds }),

  // library management
  allBooks: () => invoke('shelf_list'),
  shelfGet: (id) => invoke('shelf_get', { id }),
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

  tags: {
    list: () => invoke('list_tags'),
    create: (name, color) => invoke('tag_create', { name, color }),
    rename: (id, name) => invoke('tag_rename', { id, name }),
    delete: (id) => invoke('tag_delete', { id }),
    map: () => invoke('books_tags_bulk'),
    setOnBook: (bookId, tagIds) => invoke('book_set_tags', { bookId, tagIds }),
    setFavorite: (bookId, favorite) => invoke('book_set_favorite', { bookId, favorite }),
    duplicates: () => invoke('find_duplicate_books'),
  },
  exportBackup: (destPath) => invoke('export_library_backup', { destPath }),
  importBackup: (srcPath) => invoke('import_library_backup', { srcPath }),

  metadata: {
    getConfig: () => invoke('metadata_get_config'),
    setConfig: (cfg) => invoke('metadata_set_config', { cfg }),
    search: (bookId, query, providers = null, limit = 12) =>
      invoke('metadata_search', { bookId, query, providers, limit }),
    detail: (bookId, provider, externalId) => invoke('metadata_detail', { bookId, provider, externalId }),
    get: (bookId) => invoke('metadata_get', { bookId }),
    bind: (bookId, provider, externalId, matchKind, options) =>
      invoke('metadata_bind', { bookId, provider, externalId, matchKind, options }),
    unbind: (bookId) => invoke('metadata_unbind', { bookId }),
    automatch: (bookIds = null, dryRun = false) =>
      invoke('metadata_automatch', { bookIds, dryRun }),
    listPending: () => invoke('metadata_list_pending'),
    coverPath: (bookId) => invoke('metadata_cover_local_path', { bookId }),
    openPage: (bookId) => invoke('metadata_open_source_page', { bookId }),
    saveManual: (bookId, patch) => invoke('metadata_save_manual', { bookId, patch }),
    refresh: (bookId) => invoke('metadata_refresh', { bookId }),
    shelfIndex: () => invoke('metadata_shelf_index'),
    cacheStatus: (bookId, query = null) => invoke('metadata_cache_status', { bookId, query }),
  },
};

export { invoke };
