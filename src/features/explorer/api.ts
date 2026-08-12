import { invoke } from "@tauri-apps/api/core";

import type { DirectoryView, SearchResponse } from "./types";

export interface ExplorerApi {
  getHomeDirectory(): Promise<string>;
  readDirectory(path: string): Promise<DirectoryView>;
  watchDirectory(path: string): Promise<void>;
  searchDirectory(path: string, query: string): Promise<SearchResponse>;
  cancelSearch(): Promise<void>;
  renameEntry(path: string, newName: string): Promise<void>;
  createEntry(directory: string, name: string, kind: "file" | "directory"): Promise<string>;
  copyEntries(sources: string[], destination: string, operationId: string): Promise<void>;
  moveEntries(sources: string[], destination: string, operationId: string): Promise<void>;
  deleteEntries(paths: string[], operationId: string): Promise<void>;
}

export const explorerApi: ExplorerApi = {
  getHomeDirectory: () => invoke<string>("get_home_directory"),
  readDirectory: (path) => invoke<DirectoryView>("read_directory", { path }),
  watchDirectory: (path) => invoke<void>("watch_directory", { path }),
  searchDirectory: (path, query) => invoke<SearchResponse>("search_directory", { path, query }),
  cancelSearch: () => invoke<void>("cancel_search"),
  renameEntry: (path, newName) => invoke<void>("rename_entry", { path, newName }),
  createEntry: (directory, name, kind) =>
    invoke<string>("create_entry", { directory, name, kind }),
  copyEntries: (sources, destination, operationId) =>
    invoke<void>("copy_entries", { sources, destination, operationId }),
  moveEntries: (sources, destination, operationId) =>
    invoke<void>("move_entries", { sources, destination, operationId }),
  deleteEntries: (paths, operationId) => invoke<void>("delete_entries", { paths, operationId }),
};
