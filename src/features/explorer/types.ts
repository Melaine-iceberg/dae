export type EntryKind = "directory" | "file" | "symlink" | "other";

export interface Breadcrumb {
  name: string;
  path: string;
}

export interface DirectoryEntry {
  name: string;
  path: string;
  kind: EntryKind;
  modifiedAt: number | null;
  size: number | null;
  relativePath?: string;
}

export interface DirectoryView {
  path: string;
  breadcrumbs: Breadcrumb[];
  entries: DirectoryEntry[];
}

export interface SearchEntry extends DirectoryEntry {
  relativePath: string;
}

export interface SearchResponse {
  entries: SearchEntry[];
  truncated: boolean;
}

export type FileOperationKind = "copy" | "move" | "delete";
export type FileOperationPhase = "preparing" | "running" | "completed";

export interface FileOperationProgress {
  operationId: string;
  operation: FileOperationKind;
  phase: FileOperationPhase;
  completed: number;
  total: number | null;
  currentPath: string | null;
}

export type FileSystemErrorKind =
  | "not_found"
  | "permission_denied"
  | "not_directory"
  | "io"
  | "already_exists"
  | "invalid_input"
  | "internal";

export type FileSystemError = {
  [Kind in FileSystemErrorKind]: { kind: Kind; message: string };
}[FileSystemErrorKind];
