import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { AppWindowIcon } from "@phosphor-icons/react";

import { commands, type OpenWithApp } from "@/bindings";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { FieldError } from "@/components/ui/field";
import { Skeleton } from "@/components/ui/skeleton";
import { getFileOperationErrorMessage } from "@/i18n/errors";
import { cn } from "@/lib/utils";

const LOADING_ROW_COUNT = 6;

/**
 * In-app "Open With" picker for macOS and Linux, where the OS exposes no
 * system dialog. Lists applications registered for the item's type and can
 * open it once or register the pick as the new default handler.
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
  const [apps, setApps] = useState<OpenWithApp[] | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [setDefault, setSetDefault] = useState(false);
  const [isPending, setIsPending] = useState(false);
  const [openError, setOpenError] = useState<string | null>(null);

  const loadApps = useCallback((path: string) => {
    setApps(null);
    setLoadError(null);
    void commands
      .listOpenWithApps(path)
      .then((list) => {
        setApps(list);
        setSelectedId(list[0]?.id ?? null);
      })
      .catch((error: unknown) => {
        setLoadError(getFileOperationErrorMessage(error));
      });
  }, []);

  useEffect(() => {
    if (!target) return;

    setSetDefault(false);
    setIsPending(false);
    setOpenError(null);
    loadApps(target);
  }, [loadApps, target]);

  const targetName = target ? (target.split(/[\\/]/).filter(Boolean).pop() ?? target) : "";

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

  return (
    <Dialog onOpenChange={onOpenChange} open={target !== null}>
      <DialogContent showCloseButton={!isPending}>
        <DialogHeader>
          <DialogTitle>{t("explorer:openWith.dialogTitle")}</DialogTitle>
          <DialogDescription>
            {t("explorer:openWith.description", { name: targetName })}
          </DialogDescription>
        </DialogHeader>

        {apps === null && !loadError ? (
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
        ) : (apps ?? []).length === 0 ? (
          <p className="rounded-md border p-3 text-sm text-muted-foreground">
            {t("explorer:openWith.empty")}
          </p>
        ) : (
          <div
            aria-label={t("explorer:openWith.listAriaLabel")}
            className="max-h-64 overflow-y-auto rounded-md border p-1"
            role="radiogroup"
          >
            {(apps ?? []).map((app) => (
              <button
                aria-checked={app.id === selectedId}
                className={cn(
                  "flex w-full items-center gap-2 rounded-sm px-2 py-1.5 text-left text-sm",
                  "outline-none hover:bg-accent focus-visible:bg-accent",
                  app.id === selectedId && "bg-accent",
                )}
                disabled={isPending}
                key={app.id}
                onClick={() => setSelectedId(app.id)}
                onDoubleClick={() => confirm(app.id)}
                role="radio"
                type="button"
              >
                <AppWindowIcon className="size-4 shrink-0 text-muted-foreground" />
                <span className="truncate">{app.name}</span>
              </button>
            ))}
          </div>
        )}

        {openError && <FieldError>{openError}</FieldError>}

        <DialogFooter className="items-center sm:justify-between">
          <label className="flex items-center gap-2 text-sm text-muted-foreground select-none">
            <input
              checked={setDefault}
              className="size-4 accent-(--primary)"
              disabled={isPending || apps === null || apps.length === 0}
              onChange={(event) => setSetDefault(event.target.checked)}
              type="checkbox"
            />
            {t("explorer:openWith.setDefault")}
          </label>
          <span className="flex gap-2">
            <Button disabled={isPending} onClick={onClose} type="button" variant="outline">
              {t("explorer:actions.cancel")}
            </Button>
            <Button
              disabled={isPending || !selectedId}
              onClick={() => confirm()}
              type="button"
            >
              {t("explorer:openWith.open")}
            </Button>
          </span>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
