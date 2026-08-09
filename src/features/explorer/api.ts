import { invoke } from "@tauri-apps/api/core";

import type { DirectoryView } from "./types";

export interface ExplorerApi {
  getHomeDirectory(): Promise<string>;
  readDirectory(path: string): Promise<DirectoryView>;
  watchDirectory(path: string): Promise<void>;
}

export const explorerApi: ExplorerApi = {
  getHomeDirectory: () => invoke<string>("get_home_directory"),
  readDirectory: (path) => invoke<DirectoryView>("read_directory", { path }),
  watchDirectory: (path) => invoke<void>("watch_directory", { path }),
};
