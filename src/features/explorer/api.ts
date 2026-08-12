import { invoke } from "@tauri-apps/api/core";

import type { DirectoryView } from "./types";

export interface ExplorerApi {
  getHomeDirectory(): Promise<string>;
  readDirectory(path: string): Promise<DirectoryView>;
  watchDirectory(path: string): Promise<void>;
  renameEntry(path: string, newName: string): Promise<void>;
  copyEntries(sources: string[], destination: string, operationId: string): Promise<void>;
  moveEntries(sources: string[], destination: string, operationId: string): Promise<void>;
  deleteEntries(paths: string[], operationId: string): Promise<void>;
}

export const explorerApi: ExplorerApi = {
  getHomeDirectory: () => invoke<string>("get_home_directory"),
  readDirectory: (path) => invoke<DirectoryView>("read_directory", { path }),
  watchDirectory: (path) => invoke<void>("watch_directory", { path }),
  renameEntry: (path, newName) => invoke<void>("rename_entry", { path, newName }),
  copyEntries: (sources, destination, operationId) =>
    invoke<void>("copy_entries", { sources, destination, operationId }),
  moveEntries: (sources, destination, operationId) =>
    invoke<void>("move_entries", { sources, destination, operationId }),
  deleteEntries: (paths, operationId) => invoke<void>("delete_entries", { paths, operationId }),
};
