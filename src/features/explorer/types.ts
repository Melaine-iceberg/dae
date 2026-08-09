export type EntryKind = "directory" | "file" | "symlink" | "other";

export interface Breadcrumb {
  name: string;
  path: string;
}

export interface DirectoryEntry {
  name: string;
  path: string;
  kind: EntryKind;
}

export interface DirectoryView {
  path: string;
  breadcrumbs: Breadcrumb[];
  entries: DirectoryEntry[];
}

export type FileSystemErrorKind =
  | "not_found"
  | "permission_denied"
  | "not_directory"
  | "io"
  | "internal";

export type FileSystemError = {
  [Kind in FileSystemErrorKind]: { kind: Kind; message: string };
}[FileSystemErrorKind];
