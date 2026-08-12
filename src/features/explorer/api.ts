import { invoke } from "@tauri-apps/api/core";

import type { DirectoryView } from "./types";

export interface ExplorerApi {
  getHomeDirectory(): Promise<string>;
  readDirectory(path: string): Promise<DirectoryView>;
  watchDirectory(path: string): Promise<void>;
  renameEntry(path: string, newName: string): Promise<void>;
  copyEntries(sources: string[], destination: string): Promise<void>;
  moveEntries(sources: string[], destination: string): Promise<void>;
  deleteEntries(paths: string[]): Promise<void>;
}

export const explorerApi: ExplorerApi = {
  getHomeDirectory: () => invoke<string>("get_home_directory"),
  readDirectory: (path) => invoke<DirectoryView>("read_directory", { path }),
  watchDirectory: (path) => invoke<void>("watch_directory", { path }),
  renameEntry: (path, newName) => invoke<void>("rename_entry", { path, newName }),
  copyEntries: (sources, destination) => invoke<void>("copy_entries", { sources, destination }),
  moveEntries: (sources, destination) => invoke<void>("move_entries", { sources, destination }),
  deleteEntries: (paths) => invoke<void>("delete_entries", { paths }),
};
