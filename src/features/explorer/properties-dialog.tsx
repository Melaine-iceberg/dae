import { useEffect, useState, type ReactNode } from "react";
import { useAtomValue, useSetAtom } from "jotai";
import { useTranslation } from "react-i18next";
import {
  ArrowsClockwiseIcon,
  CheckCircleIcon,
  CheckIcon,
  CircleNotchIcon,
  CopyIcon,
  XCircleIcon,
} from "@phosphor-icons/react";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";

import { i18n } from "@/i18n";
import { localeDateTimeFormat, localeNumberFormat } from "@/i18n/format";
import { cn } from "@/lib/utils";
import {
  commands,
  events,
  type FileHashDigests,
  type FileProperties,
  type OwnerChange,
  type PropertyChanges,
} from "@/bindings";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Progress } from "@/components/ui/progress";
import { Separator } from "@/components/ui/separator";

import type { DirectoryEntry } from "./types";
import { propertiesTargetAtom } from "./properties-atoms";
import { isLocalExplorerPath } from "./drag-drop";
import {
  DIRECTORY_PRESENTATION,
  OTHER_PRESENTATION,
  SYMLINK_PRESENTATION,
  getFilePresentation,
  getPresentationIconClassName,
} from "./file-icons";

const TIMESTAMP_FORMAT_OPTIONS: Intl.DateTimeFormatOptions = {
  year: "numeric",
  month: "numeric",
  day: "numeric",
  hour: "2-digit",
  minute: "2-digit",
  second: "2-digit",
  hour12: false,
};

const BYTE_UNITS = ["B", "KB", "MB", "GB", "TB", "PB"] as const;

/** Editable state derived from a freshly loaded `FileProperties`. */
interface PropertiesDraft {
  /** Low 12 permission bits: rwx triplets plus setuid/setgid/sticky. */
  mode: number;
  user: string;
  group: string;
  readOnly: boolean;
  hidden: boolean;
  archive: boolean;
  system: boolean;
}

function createDraft(properties: FileProperties): PropertiesDraft {
  const { platform } = properties;

  return {
    mode: platform.kind === "unix" ? platform.mode & 0o7777 : 0,
    user: platform.kind === "unix" ? (platform.userName ?? String(platform.uid)) : "",
    group: platform.kind === "unix" ? (platform.groupName ?? String(platform.gid)) : "",
    readOnly: platform.kind === "windows" ? platform.readOnly : false,
    hidden: platform.kind === "windows" ? platform.hidden : false,
    archive: platform.kind === "windows" ? platform.archive : false,
    system: platform.kind === "windows" ? platform.system : false,
  };
}

/** Builds the change set for the backend: only fields that actually differ
 *  from the loaded snapshot are included, so untouched sides never run
 *  `chmod`/`chown`/`SetFileAttributesW`. */
function buildChanges(properties: FileProperties, draft: PropertiesDraft): PropertyChanges {
  const changes: PropertyChanges = {};
  const { platform } = properties;

  if (platform.kind === "unix") {
    if (draft.mode !== (platform.mode & 0o7777)) {
      changes.mode = draft.mode & 0o7777;
    }

    const owner: OwnerChange = {};
    const initialUser = platform.userName ?? String(platform.uid);
    const initialGroup = platform.groupName ?? String(platform.gid);

    if (draft.user.trim() && draft.user.trim() !== initialUser) {
      owner.user = draft.user.trim();
    }
    if (draft.group.trim() && draft.group.trim() !== initialGroup) {
      owner.group = draft.group.trim();
    }
    if (owner.user !== undefined || owner.group !== undefined) {
      changes.owner = owner;
    }
  }

  if (platform.kind === "windows") {
    if (draft.readOnly !== platform.readOnly) {
      changes.readOnly = draft.readOnly;
    }
    if (draft.hidden !== platform.hidden) {
      changes.hidden = draft.hidden;
    }
    if (draft.archive !== platform.archive) {
      changes.archive = draft.archive;
    }
    if (draft.system !== platform.system) {
      changes.system = draft.system;
    }
  }

  return changes;
}

function formatTimestamp(timestamp: number | null): string {
  return timestamp === null
    ? "—"
    : localeDateTimeFormat(TIMESTAMP_FORMAT_OPTIONS).format(new Date(timestamp));
}

function formatSize(size: number | null): string {
  if (size === null) return "—";
  if (size <= 0) return "0 B";

  const unitIndex = Math.min(Math.floor(Math.log(size) / Math.log(1024)), BYTE_UNITS.length - 1);
  const value = size / 1024 ** unitIndex;
  const formatter = localeNumberFormat({ maximumFractionDigits: 1 });
  return `${formatter.format(value)} ${BYTE_UNITS[unitIndex]}`;
}

function parentPath(path: string): string {
  const index = Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"));
  return index > 0 ? path.slice(0, index) : path;
}

function formatSymbolicMode(mode: number): string {
  const triplet = (bits: number, special: boolean, letter: string) =>
    `${bits & 4 ? "r" : "-"}${bits & 2 ? "w" : "-"}${
      bits & 1 ? (special ? letter : "x") : special ? letter.toUpperCase() : "-"
    }`;
  return (
    triplet(mode >> 6, (mode & 0o4000) !== 0, "s") +
    triplet(mode >> 3, (mode & 0o2000) !== 0, "s") +
    triplet(mode, (mode & 0o1000) !== 0, "t")
  );
}

function describeError(error: unknown): string {
  const kind =
    typeof error === "object" && error !== null && "kind" in error
      ? String((error as { kind: unknown }).kind)
      : null;

  if (kind === "unsupported") return i18n.t("explorer:properties.errorUnsupported");
  if (kind === "permission_denied") return i18n.t("explorer:properties.errorPermissionDenied");

  if (
    typeof error === "object" &&
    error !== null &&
    "message" in error &&
    typeof (error as { message: unknown }).message === "string"
  ) {
    return (error as { message: string }).message;
  }

  return error instanceof Error ? error.message : String(error);
}

type PropertiesTab = "general" | "checksum";

/** Live state of one checksum run streamed from the backend. */
interface HashRunState {
  status: "running" | "done" | "error";
  bytesRead: number;
  totalBytes: number;
  digests: FileHashDigests | null;
  error: string | null;
}

const HASH_ALGORITHMS: readonly { key: keyof FileHashDigests; label: string }[] = [
  { key: "md5", label: "MD5" },
  { key: "sha1", label: "SHA-1" },
  { key: "sha256", label: "SHA-256" },
];

/** Global properties dialog (right-click → 属性): common metadata on every
 *  platform, plus a POSIX permission matrix with special bits and an owner
 *  editor on Unix, DOS attribute toggles on Windows, and an optional
 *  "apply to enclosed items" mode for local folders. */
export function PropertiesDialog() {
  const { t } = useTranslation("explorer");
  const target = useAtomValue(propertiesTargetAtom);
  const setTarget = useSetAtom(propertiesTargetAtom);
  const [properties, setProperties] = useState<FileProperties | null>(null);
  const [draft, setDraft] = useState<PropertiesDraft | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [isSaving, setIsSaving] = useState(false);
  // Folder-only option: apply the edits to every enclosed item as well.
  const [applyToEnclosed, setApplyToEnclosed] = useState(false);
  // Live item counts streamed while a recursive apply runs.
  const [saveProgress, setSaveProgress] = useState<{
    completed: number;
    total: number | null;
  } | null>(null);
  const [tab, setTab] = useState<PropertiesTab>("general");
  // Folder sizes are computed on demand here (never in the list, to avoid
  // constant background scans). While a scan runs, the total updates live.
  const [directorySize, setDirectorySize] = useState<number | null>(null);
  const [isCalculatingDirectorySize, setIsCalculatingDirectorySize] = useState(false);

  useEffect(() => {
    if (!target) return;

    let cancelled = false;
    setProperties(null);
    setDraft(null);
    setError(null);
    setApplyToEnclosed(false);
    setSaveProgress(null);
    setTab("general");

    commands
      .getFileProperties(target.path)
      .then((result) => {
        if (cancelled) return;
        setProperties(result);
        setDraft(createDraft(result));
      })
      .catch((cause: unknown) => {
        if (!cancelled) setError(describeError(cause));
      });

    return () => {
      cancelled = true;
    };
  }, [target]);

  // Compute a folder's size only while its properties dialog is open; the
  // backend emits running totals under the operation id until the final sum.
  useEffect(() => {
    if (!target || target.kind !== "directory" || !isLocalExplorerPath(target.path)) {
      setDirectorySize(null);
      setIsCalculatingDirectorySize(false);
      return undefined;
    }

    let disposed = false;
    setDirectorySize(null);
    setIsCalculatingDirectorySize(true);

    const operationId = crypto.randomUUID();
    const unlistenPromise = events.explorerDirectorySizeProgress.listen(({ payload }) => {
      if (disposed || payload.operationId !== operationId) return;
      setDirectorySize(payload.size);
      if (payload.completed) setIsCalculatingDirectorySize(false);
    });

    void commands.startDirectorySizeCalculation(operationId, [target.path]).catch(() => {
      if (!disposed) setIsCalculatingDirectorySize(false);
    });

    return () => {
      disposed = true;
      void unlistenPromise.then((unlisten) => unlisten());
      void commands
        .cancelDirectorySizeCalculation(operationId)
        .catch((cause: unknown) => console.warn("Unable to cancel folder size scan", cause));
    };
  }, [target]);

  const close = () => setTarget(null);

  const isDirty =
    properties !== null && draft !== null
      ? Object.keys(buildChanges(properties, draft)).length > 0
      : false;

  // Folders on the local disk can push their edits into every enclosed item;
  // remote backends and view-only targets keep the single-item behavior.
  const isEnclosedApplyAvailable =
    target !== null &&
    properties !== null &&
    target.kind === "directory" &&
    isLocalExplorerPath(target.path) &&
    properties.platform.kind !== "basic";

  const applyChanges = async () => {
    if (!target || !properties || !draft || isSaving) return;

    const changes = buildChanges(properties, draft);
    if (Object.keys(changes).length === 0) return;

    setIsSaving(true);
    setError(null);

    try {
      if (isEnclosedApplyAvailable && applyToEnclosed) {
        const operationId = crypto.randomUUID();
        const unlisten = await events.explorerFileOperationProgress.listen(({ payload }) => {
          if (payload.operationId !== operationId) return;
          setSaveProgress({ completed: payload.completed, total: payload.total });
        });

        try {
          const outcome = await commands.updateFilePropertiesRecursive(
            operationId,
            target.path,
            changes,
          );
          if (outcome.failed > 0) {
            setError(t("explorer:properties.partialFailure", { count: outcome.failed }));
          }
        } finally {
          unlisten();
          setSaveProgress(null);
        }
      } else {
        await commands.updateFileProperties(target.path, changes);
      }

      const refreshed = await commands.getFileProperties(target.path);
      setProperties(refreshed);
      setDraft(createDraft(refreshed));
    } catch (cause: unknown) {
      setError(describeError(cause));
    } finally {
      setIsSaving(false);
    }
  };

  const updateDraft = (patch: Partial<PropertiesDraft>) =>
    setDraft((current) => (current ? { ...current, ...patch } : current));

  const togglePermission = (subject: "owner" | "group" | "others", bit: 4 | 2 | 1) =>
    setDraft((current) => {
      if (!current) return current;
      const shift = subject === "owner" ? 6 : subject === "group" ? 3 : 0;
      return { ...current, mode: current.mode ^ (bit << shift) };
    });

  const presentation = target ? kindPresentation(target.kind, target.name) : DIRECTORY_PRESENTATION;
  const Icon = presentation.icon;

  // Checksums only make sense for real local files; everything else keeps
  // the single-view layout without the tab strip.
  const showHashTab =
    target !== null && target.kind === "file" && isLocalExplorerPath(target.path);
  const showGeneral = !showHashTab || tab === "general";

  // Folder sizes come from the on-demand scan; everything else uses the
  // loaded snapshot. Guard `properties` since this runs before it resolves.
  const isLocalDirectoryTarget =
    target?.kind === "directory" && target !== null && isLocalExplorerPath(target.path);
  let sizeValue: ReactNode;
  if (isLocalDirectoryTarget) {
    if (directorySize === null) {
      sizeValue = (
        <span className="text-muted-foreground">{t("explorer:properties.sizeCalculating")}</span>
      );
    } else if (isCalculatingDirectorySize) {
      sizeValue = `${formatSize(directorySize)} · ${t("explorer:properties.sizeCalculating")}`;
    } else {
      const byteCount = localeNumberFormat().format(directorySize);
      sizeValue = `${formatSize(directorySize)} (${t("explorer:properties.sizeBytes", { size: byteCount })})`;
    }
  } else {
    sizeValue = formatSize(properties?.size ?? null);
  }

  return (
    <Dialog onOpenChange={(open) => !open && close()} open={target !== null}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>{t("explorer:properties.title")}</DialogTitle>
          <DialogDescription>
            {t("explorer:properties.description", { name: target?.name ?? "" })}
          </DialogDescription>
        </DialogHeader>

        {showHashTab && (
          <div className="flex gap-1 rounded-md bg-muted/60 p-0.5" role="tablist">
            <PropertiesTabButton active={showGeneral} onClick={() => setTab("general")}>
              {t("explorer:properties.tabGeneral")}
            </PropertiesTabButton>
            <PropertiesTabButton active={tab === "checksum"} onClick={() => setTab("checksum")}>
              {t("explorer:properties.tabChecksum")}
            </PropertiesTabButton>
          </div>
        )}

        {showGeneral && error && <p className="text-[13px] text-destructive">{error}</p>}

        {target && (
          <div className="flex items-center gap-3">
            <Icon
              className={cn("size-8 shrink-0", getPresentationIconClassName(presentation))}
              weight={target?.kind === "directory" ? "fill" : undefined}
            />
            <div className="min-w-0">
              <p className="truncate text-[13px] font-medium" title={target.name}>
                {target.name}
              </p>
              <p className="truncate text-xs text-muted-foreground">{presentation.label}</p>
            </div>
          </div>
        )}

        {showGeneral && target && !properties && !error && (
          <p className="text-[13px] text-muted-foreground">{t("explorer:properties.loading")}</p>
        )}

        {target && properties && draft && (
          <div className="flex flex-col gap-3" hidden={!showGeneral}>
            <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1.5 text-[13px]">
              <dt className="text-muted-foreground">{t("explorer:properties.location")}</dt>
              <dd className="truncate" title={parentPath(properties.path)}>
                {parentPath(properties.path)}
              </dd>
              <dt className="text-muted-foreground">{t("explorer:properties.size")}</dt>
              <dd>{sizeValue}</dd>
              <dt className="text-muted-foreground">{t("explorer:properties.modified")}</dt>
              <dd>{formatTimestamp(properties.modifiedAt)}</dd>
              <dt className="text-muted-foreground">{t("explorer:properties.created")}</dt>
              <dd>{formatTimestamp(properties.createdAt)}</dd>
              <dt className="text-muted-foreground">{t("explorer:properties.accessed")}</dt>
              <dd>{formatTimestamp(properties.accessedAt)}</dd>
              {properties.target && (
                <>
                  <dt className="text-muted-foreground">{t("explorer:properties.target")}</dt>
                  <dd className="truncate" title={properties.target}>
                    {properties.target}
                  </dd>
                </>
              )}
            </dl>

            <Separator />

            {properties.platform.kind === "unix" && (
              <UnixPropertiesEditor
                draft={draft}
                onTogglePermission={togglePermission}
                onUpdateDraft={updateDraft}
              />
            )}
            {properties.platform.kind === "windows" && (
              <WindowsPropertiesEditor draft={draft} onUpdateDraft={updateDraft} />
            )}
            {properties.platform.kind === "basic" && (
              <p className="text-[13px] text-muted-foreground">
                {t("explorer:properties.unsupportedPermissions")}
              </p>
            )}

            {isEnclosedApplyAvailable && (
              <label className="flex items-center gap-2 text-[13px]">
                <input
                  checked={applyToEnclosed}
                  className="size-4 accent-[var(--primary)]"
                  disabled={isSaving}
                  onChange={(event) => setApplyToEnclosed(event.target.checked)}
                  type="checkbox"
                />
                {t("explorer:properties.applyToEnclosed")}
              </label>
            )}

            {isSaving && saveProgress && (
              <p className="text-xs text-muted-foreground">
                {t("explorer:properties.applyingProgress", {
                  completed: localeNumberFormat().format(saveProgress.completed),
                  total:
                    saveProgress.total === null
                      ? "…"
                      : localeNumberFormat().format(saveProgress.total),
                })}
              </p>
            )}
          </div>
        )}

        {target && showHashTab && (
          // Keyed by path so switching targets resets the run entirely.
          <FileHashPanel active={tab === "checksum"} key={target.path} path={target.path} />
        )}

        <DialogFooter>
          <Button disabled={isSaving} onClick={close} type="button" variant="outline">
            {t("explorer:actions.close")}
          </Button>
          {showGeneral && (
            <Button disabled={!isDirty || isSaving} onClick={() => void applyChanges()} type="button">
              {isSaving ? t("explorer:properties.applying") : t("explorer:properties.apply")}
            </Button>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

const PERMISSION_SUBJECTS = [
  { key: "owner", labelKey: "permOwner", shift: 6 },
  { key: "group", labelKey: "permGroup", shift: 3 },
  { key: "others", labelKey: "permOthers", shift: 0 },
] as const;

const PERMISSION_BITS = [
  { value: 4, labelKey: "permRead" },
  { value: 2, labelKey: "permWrite" },
  { value: 1, labelKey: "permExecute" },
] as const;

const SPECIAL_BITS = [
  { value: 0o4000, labelKey: "permSetUid" },
  { value: 0o2000, labelKey: "permSetGid" },
  { value: 0o1000, labelKey: "permSticky" },
] as const;

function UnixPropertiesEditor({
  draft,
  onTogglePermission,
  onUpdateDraft,
}: {
  draft: PropertiesDraft;
  onTogglePermission: (subject: "owner" | "group" | "others", bit: 4 | 2 | 1) => void;
  onUpdateDraft: (patch: Partial<PropertiesDraft>) => void;
}) {
  const { t } = useTranslation("explorer");

  return (
    <div className="flex flex-col gap-3">
      <div className="flex items-center justify-between gap-4">
        <p className="text-[13px] font-medium">{t("explorer:properties.permissions")}</p>
        <p className="font-mono text-xs text-muted-foreground">
          {formatSymbolicMode(draft.mode)} · {draft.mode.toString(8)}
        </p>
      </div>

      <table className="w-full text-[13px]">
        <thead>
          <tr className="text-xs text-muted-foreground">
            <th className="w-1/4" />
            {PERMISSION_BITS.map(({ labelKey }) => (
              <th key={labelKey} className="font-normal">
                {t(`explorer:properties.${labelKey}`)}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {PERMISSION_SUBJECTS.map(({ key, labelKey, shift }) => {
            const subjectLabel = t(`explorer:properties.${labelKey}`);
            return (
              <tr key={key}>
                <td className="text-muted-foreground">{subjectLabel}</td>
                {PERMISSION_BITS.map(({ value, labelKey: bitLabelKey }) => (
                  <td key={bitLabelKey} className="text-center">
                    <input
                      aria-label={`${subjectLabel} · ${t(`explorer:properties.${bitLabelKey}`)}`}
                      checked={((draft.mode >> shift) & value) !== 0}
                      className="size-4 accent-[var(--primary)]"
                      onChange={() => onTogglePermission(key, value)}
                      type="checkbox"
                    />
                  </td>
                ))}
              </tr>
            );
          })}
        </tbody>
      </table>

      <div className="flex flex-col gap-1.5">
        <p className="text-xs text-muted-foreground">{t("explorer:properties.permSpecial")}</p>
        <div className="flex flex-wrap gap-x-4 gap-y-1.5">
          {SPECIAL_BITS.map(({ value, labelKey }) => (
            <label key={value} className="flex items-center gap-2 text-[13px]">
              <input
                checked={(draft.mode & value) !== 0}
                className="size-4 accent-[var(--primary)]"
                onChange={() => onUpdateDraft({ mode: draft.mode ^ value })}
                type="checkbox"
              />
              {t(`explorer:properties.${labelKey}`)}
            </label>
          ))}
        </div>
      </div>

      <div className="grid grid-cols-2 gap-3">
        <label className="flex flex-col gap-1.5">
          <span className="text-xs text-muted-foreground">
            {t("explorer:properties.ownerField")}
          </span>
          <Input
            onChange={(event) => onUpdateDraft({ user: event.target.value })}
            spellCheck={false}
            value={draft.user}
          />
        </label>
        <label className="flex flex-col gap-1.5">
          <span className="text-xs text-muted-foreground">
            {t("explorer:properties.groupField")}
          </span>
          <Input
            onChange={(event) => onUpdateDraft({ group: event.target.value })}
            spellCheck={false}
            value={draft.group}
          />
        </label>
      </div>
    </div>
  );
}

function WindowsPropertiesEditor({
  draft,
  onUpdateDraft,
}: {
  draft: PropertiesDraft;
  onUpdateDraft: (patch: Partial<PropertiesDraft>) => void;
}) {
  const { t } = useTranslation("explorer");

  return (
    <div className="flex flex-col gap-3">
      <p className="text-[13px] font-medium">{t("explorer:properties.attributes")}</p>
      <label className="flex items-center gap-2 text-[13px]">
        <input
          checked={draft.readOnly}
          className="size-4 accent-[var(--primary)]"
          onChange={(event) => onUpdateDraft({ readOnly: event.target.checked })}
          type="checkbox"
        />
        {t("explorer:properties.readOnly")}
      </label>
      <label className="flex items-center gap-2 text-[13px]">
        <input
          checked={draft.hidden}
          className="size-4 accent-[var(--primary)]"
          onChange={(event) => onUpdateDraft({ hidden: event.target.checked })}
          type="checkbox"
        />
        {t("explorer:properties.hidden")}
      </label>
      <label className="flex items-center gap-2 text-[13px]">
        <input
          checked={draft.archive}
          className="size-4 accent-[var(--primary)]"
          onChange={(event) => onUpdateDraft({ archive: event.target.checked })}
          type="checkbox"
        />
        {t("explorer:properties.archive")}
      </label>
      <label className="flex items-center gap-2 text-[13px]">
        <input
          checked={draft.system}
          className="size-4 accent-[var(--primary)]"
          onChange={(event) => onUpdateDraft({ system: event.target.checked })}
          type="checkbox"
        />
        {t("explorer:properties.system")}
      </label>
      <p className="text-xs text-muted-foreground">{t("explorer:properties.windowsAclHint")}</p>
    </div>
  );
}

function kindPresentation(kind: DirectoryEntry["kind"], name: string) {
  switch (kind) {
    case "directory":
      return DIRECTORY_PRESENTATION;
    case "symlink":
      return SYMLINK_PRESENTATION;
    case "other":
      return OTHER_PRESENTATION;
    default:
      return getFilePresentation(name);
  }
}

function PropertiesTabButton({
  active,
  children,
  onClick,
}: {
  active: boolean;
  children: ReactNode;
  onClick: () => void;
}) {
  return (
    <button
      aria-selected={active}
      className={cn(
        "flex-1 rounded-[5px] px-3 py-1 text-[13px] transition-colors duration-fast",
        active
          ? "bg-card text-foreground shadow-ambient-xs ring-1 ring-border"
          : "text-muted-foreground hover:text-foreground",
      )}
      onClick={onClick}
      role="tab"
      type="button"
    >
      {children}
    </button>
  );
}

/** Checksum tab: hashes the file once with MD5 + SHA-1 + SHA-256 in a
 *  single pass, streams progress while the read runs, then lets the user
 *  copy each digest or compare one against the value a download site
 *  published. The run only starts on the first visit to the tab, and is
 *  cancelled when the dialog closes. */
function FileHashPanel({ active, path }: { active: boolean; path: string }) {
  const { t } = useTranslation("explorer");
  const [started, setStarted] = useState(false);
  const [runId, setRunId] = useState(() => crypto.randomUUID());
  const [run, setRun] = useState<HashRunState>({
    status: "running",
    bytesRead: 0,
    totalBytes: 0,
    digests: null,
    error: null,
  });
  const [expected, setExpected] = useState("");
  const [copied, setCopied] = useState<string | null>(null);

  // Defer the run until the tab is first shown so merely opening the
  // dialog never kicks off a full-file read.
  useEffect(() => {
    if (active && !started) setStarted(true);
  }, [active, started]);

  useEffect(() => {
    if (!started) return undefined;

    let disposed = false;
    setRun({ status: "running", bytesRead: 0, totalBytes: 0, digests: null, error: null });

    const unlistenPromise = events.explorerFileHashProgress.listen(({ payload }) => {
      if (disposed || payload.operationId !== runId || payload.path !== path) return;

      if (payload.completed) {
        setRun({
          status: payload.error ? "error" : "done",
          bytesRead: payload.totalBytes,
          totalBytes: payload.totalBytes,
          digests: payload.digests,
          error: payload.error,
        });
      } else {
        setRun((current) => ({
          ...current,
          bytesRead: payload.bytesRead,
          totalBytes: payload.totalBytes,
        }));
      }
    });

    void commands.startFileHashCalculation(runId, path).catch((cause: unknown) => {
      if (!disposed) {
        setRun((current) => ({ ...current, status: "error", error: describeError(cause) }));
      }
    });

    return () => {
      disposed = true;
      void unlistenPromise.then((unlisten) => unlisten());
      void commands
        .cancelFileHashCalculation(runId)
        .catch((cause: unknown) => console.warn("Unable to cancel hash run", cause));
    };
  }, [started, runId, path]);

  const expectedNormalized = expected.trim().toLowerCase();
  const hasExpected = expectedNormalized.length > 0;
  const match = run.digests
    ? HASH_ALGORITHMS.find(({ key }) => run.digests?.[key] === expectedNormalized)
    : undefined;
  const mismatch = hasExpected && run.status === "done" && !match;
  const percent =
    run.totalBytes > 0 ? Math.min(100, Math.round((run.bytesRead / run.totalBytes) * 100)) : 0;

  const copyDigest = (algorithm: string, value: string) => {
    void writeText(value)
      .then(() => {
        setCopied(algorithm);
        window.setTimeout(() => setCopied(null), 1500);
      })
      .catch((cause) => console.warn("Unable to copy digest to clipboard", cause));
  };

  return (
    <div className="flex flex-col gap-3" hidden={!active}>
      <p className="text-xs text-muted-foreground">{t("explorer:properties.hashHint")}</p>

      {run.status === "running" && (
        <div className="flex flex-col gap-2">
          <Progress className="w-full" value={percent} />
          <p className="flex items-center gap-1.5 text-xs text-muted-foreground">
            <CircleNotchIcon className="size-3.5 animate-spin" />
            {run.totalBytes > 0
              ? `${formatSize(run.bytesRead)} / ${formatSize(run.totalBytes)} · ${percent}%`
              : t("explorer:properties.hashCalculating")}
          </p>
        </div>
      )}

      {run.status === "error" && (
        <p className="text-[13px] text-destructive">
          {t("explorer:properties.hashError")}
          {run.error ? `: ${run.error}` : ""}
        </p>
      )}

      {run.digests && (
        <>
          <dl className="flex flex-col gap-2">
            {HASH_ALGORITHMS.map(({ key, label }) => (
              <div key={key} className="grid grid-cols-[3.5rem_1fr_auto] items-center gap-2">
                <dt className="text-xs text-muted-foreground">{label}</dt>
                <dd className="min-w-0 font-mono text-xs break-all select-all">
                  {run.digests?.[key]}
                </dd>
                <button
                  aria-label={t("explorer:properties.hashCopy", { algorithm: label })}
                  className="flex size-6 shrink-0 items-center justify-center rounded-sm text-muted-foreground transition-colors duration-fast hover:bg-accent hover:text-foreground"
                  onClick={() => copyDigest(label, run.digests?.[key] ?? "")}
                  type="button"
                >
                  {copied === label ? (
                    <CheckIcon className="size-3.5" />
                  ) : (
                    <CopyIcon className="size-3.5" />
                  )}
                </button>
              </div>
            ))}
          </dl>

          <Separator />

          <label className="flex flex-col gap-1.5">
            <span className="text-xs text-muted-foreground">
              {t("explorer:properties.hashExpectedLabel")}
            </span>
            <Input
              onChange={(event) => setExpected(event.target.value)}
              placeholder={t("explorer:properties.hashExpectedPlaceholder")}
              spellCheck={false}
              value={expected}
            />
          </label>

          {match && (
            <p className="flex items-center gap-1.5 text-[13px] text-emerald-600 dark:text-emerald-400">
              <CheckCircleIcon className="size-4 shrink-0" weight="fill" />
              {t("explorer:properties.hashMatch", { algorithm: match.label })}
            </p>
          )}
          {mismatch && (
            <p className="flex items-center gap-1.5 text-[13px] text-destructive">
              <XCircleIcon className="size-4 shrink-0" weight="fill" />
              {t("explorer:properties.hashMismatch")}
            </p>
          )}
        </>
      )}

      {run.status !== "running" && (
        <Button
          className="self-start"
          disabled={!hasExpected}
          onClick={() => setRunId(crypto.randomUUID())}
          type="button"
          variant="outline"
        >
          <ArrowsClockwiseIcon className="size-3.5" />
          {t("explorer:properties.hashRecalculate")}
        </Button>
      )}
    </div>
  );
}
