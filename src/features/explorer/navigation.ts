import { explorerApi, type ExplorerApi } from "./api";
import type { Breadcrumb, DirectoryView, FileSystemError } from "./types";

export type ExplorerStatus = "idle" | "loading" | "ready" | "error";

export interface ExplorerState {
  status: ExplorerStatus;
  directory: DirectoryView | null;
  pendingPath: string | null;
  error: FileSystemError | null;
  history: string[];
  historyIndex: number;
}

export type ExplorerListener = () => void;

type NavigationMode = { type: "push" } | { type: "replace" } | { type: "history"; index: number };

const initialState: ExplorerState = {
  status: "idle",
  directory: null,
  pendingPath: null,
  error: null,
  history: [],
  historyIndex: -1,
};

const fileSystemErrorKinds = new Set<FileSystemError["kind"]>([
  "not_found",
  "permission_denied",
  "not_directory",
  "io",
  "internal",
]);

export class ExplorerNavigator {
  private state = initialState;
  private requestVersion = 0;
  private readonly listeners = new Set<ExplorerListener>();

  constructor(private readonly api: ExplorerApi = explorerApi) {}

  getSnapshot = (): ExplorerState => this.state;

  subscribe = (listener: ExplorerListener): (() => void) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };

  async initialize(): Promise<DirectoryView | undefined> {
    const requestVersion = ++this.requestVersion;
    this.setState({ ...this.state, status: "loading", pendingPath: null, error: null });

    try {
      const homeDirectory = await this.api.getHomeDirectory();

      if (requestVersion !== this.requestVersion) {
        return undefined;
      }

      return this.load(homeDirectory, { type: "replace" });
    } catch (error) {
      if (requestVersion === this.requestVersion) {
        this.setState({
          ...this.state,
          status: "error",
          pendingPath: null,
          error: toFileSystemError(error),
        });
      }

      return undefined;
    }
  }

  navigate(path: string): Promise<DirectoryView | undefined> {
    return this.load(path, { type: "push" });
  }

  async refresh(path = this.state.directory?.path): Promise<DirectoryView | undefined> {
    if (!path || this.state.directory?.path !== path || this.state.status === "loading") {
      return undefined;
    }

    const requestVersion = ++this.requestVersion;

    try {
      const directory = await this.api.readDirectory(path);

      if (requestVersion !== this.requestVersion || this.state.directory?.path !== path) {
        return undefined;
      }

      this.setState({
        ...this.state,
        status: "ready",
        directory,
        pendingPath: null,
        error: null,
      });

      return directory;
    } catch (error) {
      if (requestVersion === this.requestVersion && this.state.directory?.path === path) {
        this.setState({
          ...this.state,
          status: "error",
          pendingPath: null,
          error: toFileSystemError(error),
        });
      }

      return undefined;
    }
  }

  private async load(path: string, mode: NavigationMode): Promise<DirectoryView | undefined> {
    const requestVersion = ++this.requestVersion;
    this.setState({ ...this.state, status: "loading", pendingPath: path, error: null });

    try {
      const directory = await this.api.readDirectory(path);

      if (requestVersion !== this.requestVersion) {
        return undefined;
      }

      const history = this.updateHistory(directory.path, mode);
      this.setState({
        status: "ready",
        directory,
        pendingPath: null,
        error: null,
        ...history,
      });

      return directory;
    } catch (error) {
      if (requestVersion === this.requestVersion) {
        this.setState({
          ...this.state,
          status: "error",
          pendingPath: null,
          error: toFileSystemError(error),
        });
      }

      return undefined;
    }
  }

  navigateBreadcrumb(breadcrumb: Breadcrumb): Promise<DirectoryView | undefined> {
    return this.navigate(breadcrumb.path);
  }

  goUp(): Promise<DirectoryView | undefined> {
    const parent = this.state.directory?.breadcrumbs.at(-2);
    return parent ? this.navigateBreadcrumb(parent) : Promise.resolve(undefined);
  }

  goBack(): Promise<DirectoryView | undefined> {
    const previousIndex = this.state.historyIndex - 1;
    return previousIndex >= 0
      ? this.load(this.state.history[previousIndex], { type: "history", index: previousIndex })
      : Promise.resolve(undefined);
  }

  goForward(): Promise<DirectoryView | undefined> {
    const nextIndex = this.state.historyIndex + 1;
    return nextIndex < this.state.history.length
      ? this.load(this.state.history[nextIndex], { type: "history", index: nextIndex })
      : Promise.resolve(undefined);
  }

  private updateHistory(
    path: string,
    mode: NavigationMode,
  ): Pick<ExplorerState, "history" | "historyIndex"> {
    if (mode.type === "replace") {
      return { history: [path], historyIndex: 0 };
    }

    if (mode.type === "history") {
      const history = [...this.state.history];
      history[mode.index] = path;
      return { history, historyIndex: mode.index };
    }

    const history = this.state.history.slice(0, this.state.historyIndex + 1);

    if (history.at(-1) === path) {
      return { history, historyIndex: history.length - 1 };
    }

    history.push(path);
    return { history, historyIndex: history.length - 1 };
  }

  private setState(state: ExplorerState): void {
    this.state = state;
    this.listeners.forEach((listener) => listener());
  }
}

export const explorerNavigator = new ExplorerNavigator();

function toFileSystemError(error: unknown): FileSystemError {
  if (isFileSystemError(error)) {
    return error;
  }

  return {
    kind: "internal",
    message: error instanceof Error ? error.message : String(error),
  };
}

function isFileSystemError(value: unknown): value is FileSystemError {
  return (
    typeof value === "object" &&
    value !== null &&
    "kind" in value &&
    "message" in value &&
    typeof value.kind === "string" &&
    fileSystemErrorKinds.has(value.kind as FileSystemError["kind"]) &&
    typeof value.message === "string"
  );
}
