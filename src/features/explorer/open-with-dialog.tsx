import { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { MagnifierIcon, WindowFrameIcon } from "@solar-icons/react/line-duotone";

import { commands, type OpenWithChoices } from "@/bindings";

import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { FieldError } from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import { Skeleton } from "@/components/ui/skeleton";
import { getFileOperationErrorMessage } from "@/i18n/errors";
import { cn } from "@/lib/utils";

import { buildNamedIconUrl, devicePixelSize } from "./native-icon";

const LOADING_ROW_COUNT = 6;

/**
 * In-app "Open With" picker for macOS and Linux, where the OS exposes no
 * system dialog. Shows the desktop's own answer to "what can open this" — the
 * current default, the applications that advertise the type, and everything
 * else the desktop would offer — and opens the item once or registers the pick
 * as the new default handler.
 */
export function OpenWithDialog({
  onClose,
  onOpenChange,
  target,
}: {
  onClose: () => void;
  onOpenChange: (open: boolean) => void;
  target: string | null;
}) {
  const { t } = useTranslation("explorer");
  const [choices, setChoices] = useState<OpenWithChoices | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [setDefault, setSetDefault] = useState(false);
  const [isPending, setIsPending] = useState(false);
  const [openError, setOpenError] = useState<string | null>(null);

  const loadApps = useCallback((path: string) => {
    setChoices(null);
    setLoadError(null);
    void commands
      .listOpenWithApps(path)
      .then((result) => {
        setChoices(result);
        setSelectedId(
          result.default?.id ?? result.recommended[0]?.id ?? result.other[0]?.id ?? null,
        );
      })
      .catch((error: unknown) => {
        setLoadError(getFileOperationErrorMessage(error));
      });
  }, []);

  useEffect(() => {
    if (!target) return;

    setQuery("");
    setSetDefault(false);
    setIsPending(false);
    setOpenError(null);
    loadApps(target);
  }, [loadApps, target]);

  const targetName = target ? (target.split(/[\\/]/).filter(Boolean).pop() ?? target) : "";

  // The groups are the backend's, and so is the duplication between them: the
  // desktop draws the default app twice on purpose, once to say what will happen
  // and once to say it was chosen. Filtering keeps that, so a search for the
  // current default lights up two rows.
  const sections = useMemo(() => {
    if (!choices) {
      return [];
    }
    const needle = query.trim().toLowerCase();
    return [
      {
        key: "default",
        label: t("explorer:openWith.defaultSection"),
        apps: choices.default ? [choices.default] : [],
      },
      {
        key: "recommended",
        label: t("explorer:openWith.recommendedSection"),
        apps: choices.recommended,
      },
      { key: "other", label: t("explorer:openWith.otherSection"), apps: choices.other },
    ]
      .map((section) => ({
        ...section,
        apps: needle
          ? section.apps.filter((app) => app.name.toLowerCase().includes(needle))
          : section.apps,
      }))
      .filter((section) => section.apps.length > 0);
  }, [choices, query, t]);

  const confirm = useCallback(
    (appId?: string) => {
      const id = appId ?? selectedId;
      if (!target || !id || isPending) return;

      setIsPending(true);
      setOpenError(null);
      void commands
        .openWithApp(target, id, setDefault)
        .then(() => onClose())
        .catch((error: unknown) => {
          setOpenError(getFileOperationErrorMessage(error));
          setIsPending(false);
        });
    },
    [isPending, onClose, selectedId, setDefault, target],
  );

  const totalRows = (choices?.recommended.length ?? 0) + (choices?.other.length ?? 0);
  // Only the platform with an unfiltered tail list needs a filter for it, and
  // only a list long enough to scroll needs the field before it can be used.
  const needsFilter = (choices?.other.length ?? 0) > 0 && totalRows > 8;

  return (
    <Dialog onOpenChange={onOpenChange} open={target !== null}>
      <DialogContent showCloseButton={!isPending}>
        <DialogHeader>
          <DialogTitle>{t("explorer:openWith.dialogTitle")}</DialogTitle>
          <DialogDescription>
            {t("explorer:openWith.description", { name: targetName })}
          </DialogDescription>
        </DialogHeader>

        {choices === null && !loadError ? (
          <div className="flex flex-col gap-1 rounded-md border p-1" role="status">
            {Array.from({ length: LOADING_ROW_COUNT }, (_, index) => (
              <Skeleton key={index} className="h-8 w-full" />
            ))}
          </div>
        ) : loadError ? (
          <div className="flex flex-col items-start gap-2 rounded-md border p-3">
            <FieldError>{loadError}</FieldError>
            <Button
              disabled={isPending}
              onClick={() => target && loadApps(target)}
              size="xs"
              type="button"
              variant="outline"
            >
              {t("explorer:actions.retry")}
            </Button>
          </div>
        ) : (
          <div className="flex flex-col gap-2">
            {needsFilter && (
              <div className="relative">
                <MagnifierIcon className="pointer-events-none absolute left-2 top-1/2 size-3.5 -translate-y-1/2 text-foreground/72" />
                <Input
                  aria-label={t("explorer:openWith.filterAriaLabel")}
                  className="pl-7"
                  disabled={isPending}
                  onChange={(event) => setQuery(event.target.value)}
                  placeholder={t("explorer:openWith.filterPlaceholder")}
                  type="search"
                  value={query}
                />
              </div>
            )}
            {sections.length === 0 ? (
              <p className="rounded-md border p-3 text-caption text-muted-foreground">
                {query ? t("explorer:openWith.noMatch", { query }) : t("explorer:openWith.empty")}
              </p>
            ) : (
              <div
                aria-label={t("explorer:openWith.listAriaLabel")}
                className="max-h-72 overflow-y-auto rounded-md border p-1"
                role="radiogroup"
              >
                {sections.map((section) => (
                  <div key={section.key}>
                    <p className="px-2 pt-1.5 pb-1 text-caption font-medium text-muted-foreground">
                      {section.label}
                    </p>
                    {section.apps.map((app) => (
                      <button
                        aria-checked={app.id === selectedId}
                        className={cn(
                          "flex w-full items-center gap-2 rounded-sm px-2 py-1.5 text-left text-body",
                          "outline-none hover:bg-accent focus-visible:bg-accent",
                          app.id === selectedId && "bg-accent",
                        )}
                        disabled={isPending}
                        key={`${section.key}-${app.id}`}
                        onClick={() => setSelectedId(app.id)}
                        onDoubleClick={() => confirm(app.id)}
                        role="radio"
                        type="button"
                      >
                        <AppIcon name={app.iconName} />
                        <span className="truncate">{app.name}</span>
                      </button>
                    ))}
                  </div>
                ))}
              </div>
            )}
          </div>
        )}

        {openError && <FieldError>{openError}</FieldError>}

        <DialogFooter className="items-center sm:justify-between">
          <label className="flex items-center gap-2 text-body text-muted-foreground select-none">
            <Checkbox
              checked={setDefault}
              disabled={isPending || !selectedId}
              onCheckedChange={setSetDefault}
            />
            {t("explorer:openWith.setDefault")}
          </label>
          <span className="flex gap-2">
            <Button disabled={isPending} onClick={onClose} type="button" variant="outline">
              {t("explorer:actions.cancel")}
            </Button>
            <Button disabled={isPending || !selectedId} onClick={() => confirm()} type="button">
              {t("explorer:openWith.open")}
            </Button>
          </span>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

/**
 * The icon the desktop would draw for an application, fetched from the icon
 * theme through `fileicon://` rather than sent with the list — a folder's worth
 * of theme icons base64-encoded is megabytes of `invoke` payload for a dialog
 * that shows a dozen at a time.
 *
 * The failure is per row and silent on purpose: a name no theme answers, or a
 * platform with no theme at all, keeps the drawn glyph.
 */
function AppIcon({ name }: { name: string | null }) {
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    setFailed(false);
  }, [name]);

  if (!name || failed) {
    return <WindowFrameIcon className="size-4 shrink-0 text-muted-foreground" />;
  }

  return (
    <img
      alt=""
      className="size-4 shrink-0"
      crossOrigin="anonymous"
      decoding="async"
      draggable={false}
      onError={() => setFailed(true)}
      // The row paints 16 CSS px; asking for the device pixels behind them is
      // what lets the served bitmap land 1:1 instead of resampled.
      src={buildNamedIconUrl(name, devicePixelSize(16))}
    />
  );
}
