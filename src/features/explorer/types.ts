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
import type { DirectoryEntry as GeneratedDirectoryEntry, EntryKind } from "@/bindings";

// Search results carry a relative location; regular directory listings do not.
export type DirectoryEntry = GeneratedDirectoryEntry & { relativePath?: string };

/**
 * The fields every piece of entry artwork needs, and nothing else.
 *
 * A listing row has a full `DirectoryEntry`; a recent item, a content-search
 * hit, a trash row and one side of a transfer conflict each carry their own
 * slice of it. Naming the slice here lets every one of them reach the same
 * `thumbnail://` / `fileicon://` pipeline without fabricating a listing entry —
 * and without a second, drifting copy of the URL/version rules.
 *
 * `size` and `modifiedAt` stay optional because not every source has them;
 * they only version the URL so a replaced file reads as a new one.
 */
export interface ArtworkEntry {
  path: string;
  name: string;
  kind: EntryKind;
  size?: number | null;
  modifiedAt?: number | null;
}
