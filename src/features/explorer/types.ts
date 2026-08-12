export type {
  Breadcrumb,
  DirectoryView,
  EntryKind,
  FileOperationKind,
  FileOperationPhase,
  FileOperationProgress,
  FileSystemError,
  NewEntryKind,
  SearchEntry,
  SearchResponse,
} from "@/bindings";
import type { DirectoryEntry as GeneratedDirectoryEntry } from "@/bindings";

// Search results carry a relative location; regular directory listings do not.
export type DirectoryEntry = GeneratedDirectoryEntry & { relativePath?: string };
