import { useEffect, useState } from "react";
import { atom, useAtomValue, useSetAtom } from "jotai";
import { useTranslation } from "react-i18next";

import { i18n } from "@/i18n";
import { localeDateTimeFormat, localeNumberFormat } from "@/i18n/format";
import { commands, type FileProperties, type OwnerChange, type PropertyChanges } from "@/bindings";
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
import { Separator } from "@/components/ui/separator";

import type { DirectoryEntry } from "./types";
import {
  DIRECTORY_PRESENTATION,
  OTHER_PRESENTATION,
  SYMLINK_PRESENTATION,
  getFilePresentation,
} from "./file-icons";

/** The entry whose properties dialog is open; `null` keeps the dialog closed.
 *  Kept in a global atom so any context menu (list/grid/columns) can open it
 *  without threading callbacks through every view. */
export const propertiesTargetAtom = atom<DirectoryEntry | null>(null);

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
  /** Low 9 permission bits; setuid/setgid/sticky are preserved, not edited. */
  mode: number;
  user: string;
  group: string;
  readOnly: boolean;
  hidden: boolean;
}

function createDraft(properties: FileProperties): PropertiesDraft {
  const { platform } = properties;

  return {
    mode: platform.kind === "unix" ? platform.mode & 0o777 : 0,
    user: platform.kind === "unix" ? (platform.userName ?? String(platform.uid)) : "",
    group: platform.kind === "unix" ? (platform.groupName ?? String(platform.gid)) : "",
    readOnly: platform.kind === "windows" ? platform.readOnly : false,
    hidden: platform.kind === "windows" ? platform.hidden : false,
  };
}

/** Builds the change set for the backend: only fields that actually differ
 *  from the loaded snapshot are included, so untouched sides never run
 *  `chmod`/`chown`/`SetFileAttributesW`. */
function buildChanges(properties: FileProperties, draft: PropertiesDraft): PropertyChanges {
  const changes: PropertyChanges = {};
  const { platform } = properties;

  if (platform.kind === "unix") {
    if (draft.mode !== (platform.mode & 0o777)) {
      changes.mode = (platform.mode & 0o7000) | draft.mode;
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
  const triplet = (bits: number) =>
    `${bits & 4 ? "r" : "-"}${bits & 2 ? "w" : "-"}${bits & 1 ? "x" : "-"}`;
  return `${triplet(mode >> 6)}${triplet(mode >> 3)}${triplet(mode)}`;
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

/** Global properties dialog (right-click → 属性): common metadata on every
 *  platform, plus a POSIX permission matrix / owner editor on Unix and
 *  read-only/hidden toggles on Windows. */
export function PropertiesDialog() {
  const { t } = useTranslation("explorer");
  const target = useAtomValue(propertiesTargetAtom);
  const setTarget = useSetAtom(propertiesTargetAtom);
  const [properties, setProperties] = useState<FileProperties | null>(null);
  const [draft, setDraft] = useState<PropertiesDraft | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [isSaving, setIsSaving] = useState(false);

  useEffect(() => {
    if (!target) return;

    let cancelled = false;
    setProperties(null);
    setDraft(null);
    setError(null);

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

  const close = () => setTarget(null);

  const isDirty =
    properties !== null && draft !== null
      ? Object.keys(buildChanges(properties, draft)).length > 0
      : false;

  const applyChanges = async () => {
    if (!target || !properties || !draft || isSaving) return;

    const changes = buildChanges(properties, draft);
    if (Object.keys(changes).length === 0) return;

    setIsSaving(true);
    setError(null);

    try {
      await commands.updateFileProperties(target.path, changes);
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

  const presentation = target
    ? kindPresentation(target.kind, target.name)
    : DIRECTORY_PRESENTATION;
  const Icon = presentation.icon;

  return (
    <Dialog onOpenChange={(open) => !open && close()} open={target !== null}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>{t("explorer:properties.title")}</DialogTitle>
          <DialogDescription>
            {t("explorer:properties.description", { name: target?.name ?? "" })}
          </DialogDescription>
        </DialogHeader>

        {error && <p className="text-[13px] text-destructive">{error}</p>}

        {target && (
          <div className="flex items-center gap-3">
            <Icon className="size-8 shrink-0 text-muted-foreground" />
            <div className="min-w-0">
              <p className="truncate text-[13px] font-medium" title={target.name}>
                {target.name}
              </p>
              <p className="truncate text-xs text-muted-foreground">{presentation.label}</p>
            </div>
          </div>
        )}

        {target && !properties && !error && (
          <p className="text-[13px] text-muted-foreground">{t("explorer:properties.loading")}</p>
        )}

        {target && properties && draft && (
          <>
            <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1.5 text-[13px]">
              <dt className="text-muted-foreground">{t("explorer:properties.location")}</dt>
              <dd className="truncate" title={parentPath(properties.path)}>
                {parentPath(properties.path)}
              </dd>
              <dt className="text-muted-foreground">{t("explorer:properties.size")}</dt>
              <dd>{formatSize(properties.size)}</dd>
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
          </>
        )}

        <DialogFooter>
          <Button disabled={isSaving} onClick={close} type="button" variant="outline">
            {t("explorer:actions.close")}
          </Button>
          <Button
            disabled={!isDirty || isSaving}
            onClick={() => void applyChanges()}
            type="button"
          >
            {isSaving ? t("explorer:properties.applying") : t("explorer:properties.apply")}
          </Button>
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
