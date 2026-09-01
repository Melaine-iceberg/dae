import { useEffect, useMemo, useState, type FormEvent } from "react";
import { useTranslation } from "react-i18next";
import { ArrowsLeftRightIcon } from "@phosphor-icons/react";

import type { RenameRequest } from "@/bindings";

import { isWindowsPlatform } from "@/lib/platform";
import { cn } from "@/lib/utils";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Field, FieldError, FieldGroup, FieldLabel } from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";

import type { DirectoryEntry } from "./types";

export type BulkRenameMode = "replace" | "sequence" | "case";
export type CaseTransform = "lower" | "upper" | "title" | "sentence";

export interface BulkRenameOptions {
  mode: BulkRenameMode;
  replace: {
    find: string;
    replacement: string;
    caseSensitive: boolean;
    useRegex: boolean;
  };
  sequence: {
    baseName: string;
    start: number;
    step: number;
    padding: number;
  };
  case: { transform: CaseTransform };
}

export const DEFAULT_BULK_RENAME_OPTIONS: BulkRenameOptions = {
  mode: "sequence",
  replace: { find: "", replacement: "", caseSensitive: false, useRegex: false },
  sequence: { baseName: "", start: 1, step: 1, padding: 2 },
  case: { transform: "lower" },
};

export type BulkRenameItemStatus = "rename" | "unchanged" | "error";

export interface BulkRenamePlanItem {
  entry: DirectoryEntry;
  newName: string;
  status: BulkRenameItemStatus;
  /** i18n key suffix under `bulkRename.errors` when status is "error". */
  errorKey: "empty" | "invalid" | "duplicate" | "exists" | null;
}

export interface BulkRenamePlan {
  /** Set when the whole plan cannot be computed (e.g. broken regex). */
  globalErrorKey: "regexInvalid" | null;
  items: BulkRenamePlanItem[];
}

/** Windows rejects these characters anywhere in a name, on every platform we
 *  preview for; the backend enforces the real per-OS rules on apply. */
const INVALID_NAME_PATTERN = /[<>:"/\\|?*]|\p{C}/u;
/** Windows additionally refuses names that end in a space or a dot. */
const WINDOWS_TRAILING_PATTERN = /[ .]+$/;

/** Huge selections (Ctrl+A in a big folder) only preview their head; the
 *  plan itself always covers every entry. */
const PREVIEW_ROW_LIMIT = 200;

/** Splits a display name into base and extension; leading dots belong to
 *  the base (".gitignore" has no extension). */
function splitName(name: string): { base: string; ext: string } {
  const dotIndex = name.lastIndexOf(".");
  if (dotIndex <= 0) return { base: name, ext: "" };
  return { base: name.slice(0, dotIndex), ext: name.slice(dotIndex) };
}

function escapeRegExp(text: string): string {
  return text.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

function padNumber(value: number, padding: number): string {
  const sign = value < 0 ? "-" : "";
  return sign + String(Math.abs(value)).padStart(Math.max(0, padding), "0");
}

function transformCase(text: string, transform: CaseTransform): string {
  switch (transform) {
    case "lower":
      return text.toLocaleLowerCase();
    case "upper":
      return text.toLocaleUpperCase();
    case "title":
      return text
        .toLocaleLowerCase()
        .replace(/(^|\s)(\p{L})/gu, (_, lead, letter) => lead + letter.toLocaleUpperCase());
    case "sentence":
      return text.replace(/^(\s*)(\p{L})/u, (_, lead, letter) => lead + letter.toLocaleUpperCase());
  }
}

/** Computes the proposed new name for one entry under `options`; returns
 *  null when a mode-level regex is broken for every entry. */
function computeNewName(
  entry: DirectoryEntry,
  index: number,
  options: BulkRenameOptions,
): string | null {
  const { base, ext } = splitName(entry.name);

  switch (options.mode) {
    case "replace": {
      const { find, replacement, caseSensitive, useRegex } = options.replace;
      if (!find) return entry.name;
      let pattern: RegExp;
      try {
        pattern = new RegExp(useRegex ? find : escapeRegExp(find), caseSensitive ? "g" : "gi");
      } catch {
        return null;
      }
      return base.replace(pattern, replacement) + ext;
    }
    case "sequence": {
      const { baseName, start, step, padding } = options.sequence;
      const number = start + index * step;
      return baseName + padNumber(number, padding) + ext;
    }
    case "case":
      return transformCase(base, options.case.transform) + ext;
  }
}

/** Pure preview engine: names every entry under the current options and
 *  flags empty/invalid names, duplicates inside the batch, and collisions
 *  with existing directory entries (case-insensitive, like Windows). */
export function planBulkRename(
  entries: DirectoryEntry[],
  existingNames: string[],
  options: BulkRenameOptions,
): BulkRenamePlan {
  const existing = new Set(existingNames.map((name) => name.toLocaleLowerCase()));
  const takenInBatch = new Set<string>();
  const items: BulkRenamePlanItem[] = [];

  for (let index = 0; index < entries.length; index += 1) {
    const entry = entries[index];
    const newName = computeNewName(entry, index, options);
    if (newName === null) {
      return { globalErrorKey: "regexInvalid", items: [] };
    }

    let errorKey: BulkRenamePlanItem["errorKey"] = null;
    const key = newName.toLocaleLowerCase();
    const selfKey = entry.name.toLocaleLowerCase();

    if (!newName.trim() || newName === "." || newName === "..") {
      errorKey = "empty";
    } else if (
      INVALID_NAME_PATTERN.test(newName) ||
      (isWindowsPlatform && WINDOWS_TRAILING_PATTERN.test(newName))
    ) {
      errorKey = "invalid";
    } else if (takenInBatch.has(key)) {
      errorKey = "duplicate";
    } else if (existing.has(key) && key !== selfKey) {
      errorKey = "exists";
    }

    if (!errorKey) takenInBatch.add(key);

    items.push({
      entry,
      newName,
      status: errorKey ? "error" : newName === entry.name ? "unchanged" : "rename",
      errorKey,
    });
  }

  return { globalErrorKey: null, items };
}

/** Dialog offering patterned batch renames (replace / numbering / case) for
 *  a multi-selection, with a live preview and conflict detection. */
export function BulkRenameDialog({
  applyError,
  entries,
  existingNames,
  isPending,
  onApply,
  onClose,
  onOpenChange,
  open,
}: {
  applyError: string | null;
  entries: DirectoryEntry[];
  existingNames: string[];
  isPending: boolean;
  onApply: (requests: RenameRequest[]) => void;
  onClose: () => void;
  onOpenChange: (open: boolean) => void;
  open: boolean;
}) {
  const { t } = useTranslation("explorer");
  const [options, setOptions] = useState<BulkRenameOptions>(DEFAULT_BULK_RENAME_OPTIONS);

  // A fresh selection starts a fresh session; options carry no meaning
  // across different directories.
  useEffect(() => {
    if (open) setOptions(DEFAULT_BULK_RENAME_OPTIONS);
  }, [open]);

  const plan = useMemo(
    () => planBulkRename(entries, existingNames, options),
    [entries, existingNames, options],
  );
  const renameCount = plan.items.filter((item) => item.status === "rename").length;
  const errorCount = plan.items.filter((item) => item.status === "error").length;
  const canApply = renameCount > 0 && errorCount === 0 && !plan.globalErrorKey && !isPending;

  const submit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (!canApply) return;
    onApply(
      plan.items
        .filter((item) => item.status === "rename")
        .map((item) => ({ path: item.entry.path, newName: item.newName })),
    );
  };

  const setMode = (mode: BulkRenameMode) => {
    setOptions((current) => ({ ...current, mode }));
  };

  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <DialogContent className="sm:max-w-xl" showCloseButton={!isPending}>
        <DialogHeader>
          <DialogTitle>{t("explorer:bulkRename.title")}</DialogTitle>
          <DialogDescription>
            {t("explorer:bulkRename.description", { number: entries.length })}
          </DialogDescription>
        </DialogHeader>
        <form className="flex flex-col gap-4" onSubmit={submit}>
          <div
            aria-label={t("explorer:bulkRename.modeLabel")}
            className="flex gap-1"
            role="tablist"
          >
            {(["sequence", "replace", "case"] as const).map((mode) => (
              <Button
                aria-selected={options.mode === mode}
                disabled={isPending}
                key={mode}
                onClick={() => setMode(mode)}
                role="tab"
                size="sm"
                type="button"
                variant={options.mode === mode ? "secondary" : "ghost"}
              >
                {t(`explorer:bulkRename.modes.${mode}`)}
              </Button>
            ))}
          </div>

          <FieldGroup>
            {options.mode === "sequence" && (
              <>
                <Field>
                  <FieldLabel htmlFor="bulk-rename-base">
                    {t("explorer:bulkRename.sequence.baseName")}
                  </FieldLabel>
                  <Input
                    autoFocus
                    disabled={isPending}
                    id="bulk-rename-base"
                    onChange={(event) =>
                      setOptions((current) => ({
                        ...current,
                        sequence: { ...current.sequence, baseName: event.target.value },
                      }))
                    }
                    placeholder={t("explorer:bulkRename.sequence.baseNamePlaceholder")}
                    value={options.sequence.baseName}
                  />
                </Field>
                <div className="grid grid-cols-3 gap-3">
                  <Field>
                    <FieldLabel htmlFor="bulk-rename-start">
                      {t("explorer:bulkRename.sequence.start")}
                    </FieldLabel>
                    <Input
                      disabled={isPending}
                      id="bulk-rename-start"
                      min={0}
                      onChange={(event) =>
                        setOptions((current) => ({
                          ...current,
                          sequence: {
                            ...current.sequence,
                            start: parseNumberField(event.target.value, 0),
                          },
                        }))
                      }
                      type="number"
                      value={options.sequence.start}
                    />
                  </Field>
                  <Field>
                    <FieldLabel htmlFor="bulk-rename-step">
                      {t("explorer:bulkRename.sequence.step")}
                    </FieldLabel>
                    <Input
                      disabled={isPending}
                      id="bulk-rename-step"
                      min={1}
                      onChange={(event) =>
                        setOptions((current) => ({
                          ...current,
                          sequence: {
                            ...current.sequence,
                            step: Math.max(1, parseNumberField(event.target.value, 1)),
                          },
                        }))
                      }
                      type="number"
                      value={options.sequence.step}
                    />
                  </Field>
                  <Field>
                    <FieldLabel htmlFor="bulk-rename-padding">
                      {t("explorer:bulkRename.sequence.padding")}
                    </FieldLabel>
                    <Input
                      disabled={isPending}
                      id="bulk-rename-padding"
                      max={8}
                      min={0}
                      onChange={(event) =>
                        setOptions((current) => ({
                          ...current,
                          sequence: {
                            ...current.sequence,
                            padding: Math.min(
                              8,
                              Math.max(0, parseNumberField(event.target.value, 0)),
                            ),
                          },
                        }))
                      }
                      type="number"
                      value={options.sequence.padding}
                    />
                  </Field>
                </div>
              </>
            )}

            {options.mode === "replace" && (
              <>
                <div className="grid grid-cols-2 gap-3">
                  <Field>
                    <FieldLabel htmlFor="bulk-rename-find">
                      {t("explorer:bulkRename.replace.find")}
                    </FieldLabel>
                    <Input
                      autoFocus
                      disabled={isPending}
                      id="bulk-rename-find"
                      onChange={(event) =>
                        setOptions((current) => ({
                          ...current,
                          replace: { ...current.replace, find: event.target.value },
                        }))
                      }
                      value={options.replace.find}
                    />
                  </Field>
                  <Field>
                    <FieldLabel htmlFor="bulk-rename-replacement">
                      {t("explorer:bulkRename.replace.replacement")}
                    </FieldLabel>
                    <Input
                      disabled={isPending}
                      id="bulk-rename-replacement"
                      onChange={(event) =>
                        setOptions((current) => ({
                          ...current,
                          replace: { ...current.replace, replacement: event.target.value },
                        }))
                      }
                      value={options.replace.replacement}
                    />
                  </Field>
                </div>
                <div className="flex gap-4 text-[13px] text-muted-foreground">
                  <label className="flex items-center gap-1.5">
                    <input
                      checked={options.replace.useRegex}
                      className="accent-primary"
                      disabled={isPending}
                      onChange={(event) =>
                        setOptions((current) => ({
                          ...current,
                          replace: { ...current.replace, useRegex: event.target.checked },
                        }))
                      }
                      type="checkbox"
                    />
                    {t("explorer:bulkRename.replace.useRegex")}
                  </label>
                  <label
                    className={cn(
                      "flex items-center gap-1.5",
                      options.replace.useRegex && "opacity-50",
                    )}
                  >
                    <input
                      checked={options.replace.caseSensitive}
                      className="accent-primary"
                      disabled={isPending || options.replace.useRegex}
                      onChange={(event) =>
                        setOptions((current) => ({
                          ...current,
                          replace: { ...current.replace, caseSensitive: event.target.checked },
                        }))
                      }
                      type="checkbox"
                    />
                    {t("explorer:bulkRename.replace.caseSensitive")}
                  </label>
                </div>
              </>
            )}

            {options.mode === "case" && (
              <Field>
                <FieldLabel>{t("explorer:bulkRename.case.label")}</FieldLabel>
                <Select
                  disabled={isPending}
                  onValueChange={(transform) =>
                    setOptions((current) => ({
                      ...current,
                      case: { transform: transform as CaseTransform },
                    }))
                  }
                  value={options.case.transform}
                >
                  <SelectTrigger autoFocus>
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {(["lower", "upper", "title", "sentence"] as const).map((transform) => (
                      <SelectItem key={transform} value={transform}>
                        {t(`explorer:bulkRename.case.${transform}`)}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </Field>
            )}

            <Field data-invalid={Boolean(applyError || plan.globalErrorKey)}>
              <FieldError>
                {applyError ??
                  (plan.globalErrorKey
                    ? t(`explorer:bulkRename.errors.${plan.globalErrorKey}`)
                    : "")}
              </FieldError>
            </Field>
          </FieldGroup>

          <div className="max-h-56 overflow-y-auto rounded-md border bg-muted/30 px-3 py-2">
            <div className="grid grid-cols-[1fr_auto_1fr] items-center gap-2 border-b pb-1 text-[11px] font-medium text-muted-foreground">
              <span>{t("explorer:bulkRename.preview.original")}</span>
              <span aria-hidden="true" />
              <span>{t("explorer:bulkRename.preview.newName")}</span>
            </div>
            {plan.items.slice(0, PREVIEW_ROW_LIMIT).map((item) => (
              <div
                className="grid grid-cols-[1fr_auto_1fr] items-center gap-2 py-1 text-[13px]"
                key={item.entry.path}
              >
                <span className="truncate text-muted-foreground" title={item.entry.name}>
                  {item.entry.name}
                </span>
                <ArrowsLeftRightIcon
                  className={cn(
                    "size-3 shrink-0",
                    item.status === "error"
                      ? "text-destructive"
                      : item.status === "rename"
                        ? "text-primary"
                        : "text-muted-foreground/60",
                  )}
                />
                <span
                  className={cn(
                    "truncate",
                    item.status === "error" && "text-destructive",
                    item.status === "unchanged" && "text-muted-foreground/60",
                  )}
                  title={
                    item.status === "error" && item.errorKey
                      ? t(`explorer:bulkRename.errors.${item.errorKey}`, { name: item.newName })
                      : item.newName
                  }
                >
                  {item.status === "unchanged"
                    ? t("explorer:bulkRename.preview.unchanged")
                    : item.newName || t("explorer:bulkRename.errors.empty")}
                </span>
              </div>
            ))}
            {plan.items.length > PREVIEW_ROW_LIMIT && (
              <div className="py-1 text-[12px] text-muted-foreground">
                {t("explorer:bulkRename.preview.more", {
                  hidden: plan.items.length - PREVIEW_ROW_LIMIT,
                })}
              </div>
            )}
          </div>

          <DialogFooter className="items-center gap-2 sm:justify-between">
            <span className="text-xs text-muted-foreground">
              {t("explorer:bulkRename.summary", { rename: renameCount, errors: errorCount })}
            </span>
            <div className="flex gap-2">
              <Button disabled={isPending} onClick={onClose} type="button" variant="outline">
                {t("explorer:actions.cancel")}
              </Button>
              <Button disabled={!canApply} type="submit">
                {t("explorer:bulkRename.apply", { number: renameCount })}
              </Button>
            </div>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

/** Numeric inputs stay controlled even while the field is mid-edit. */
function parseNumberField(text: string, fallback: number): number {
  const parsed = Number.parseInt(text, 10);
  return Number.isFinite(parsed) ? parsed : fallback;
}
