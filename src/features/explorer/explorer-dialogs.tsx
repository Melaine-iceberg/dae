/**
 * Presentational dialogs driven by `useEntryDialogs` and the entry actions in
 * `explorer-view.tsx`.
 *
 * Every component here is props-only: open/close, validation and submission
 * live with the caller, so these can be reasoned about (and reused) without
 * the view's state machinery.
 */
import type { FormEvent } from "react";
import { useTranslation } from "react-i18next";
import { TriangleAlert } from "lucide-react";

import { Alert, AlertAction, AlertDescription, AlertTitle } from "@/components/ui/alert";
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

import type { DirectoryEntry, NewEntryKind } from "./types";

export function RenameDialog({
  error,
  isPending,
  onClose,
  onOpenChange,
  onSubmit,
  onValueChange,
  target,
  value,
}: {
  error: string | null;
  isPending: boolean;
  onClose: () => void;
  onOpenChange: (open: boolean) => void;
  onSubmit: (event: FormEvent<HTMLFormElement>) => void;
  onValueChange: (value: string) => void;
  target: DirectoryEntry | null;
  value: string;
}) {
  const { t } = useTranslation("explorer");

  return (
    <Dialog onOpenChange={onOpenChange} open={target !== null}>
      <DialogContent showCloseButton={!isPending}>
        <DialogHeader>
          <DialogTitle>{t("explorer:rename.title")}</DialogTitle>
          <DialogDescription>
            {t("explorer:rename.description", { name: target?.name ?? "" })}
          </DialogDescription>
        </DialogHeader>
        <form className="flex flex-col gap-4" onSubmit={onSubmit}>
          <FieldGroup>
            <Field data-invalid={Boolean(error)}>
              <FieldLabel htmlFor="rename-entry">{t("explorer:rename.newNameLabel")}</FieldLabel>
              <Input
                aria-invalid={Boolean(error)}
                autoFocus
                disabled={isPending}
                id="rename-entry"
                onChange={(event) => onValueChange(event.target.value)}
                onFocus={(event) => event.currentTarget.select()}
                value={value}
              />
              <FieldError>{error}</FieldError>
            </Field>
          </FieldGroup>
          <DialogFooter>
            <Button disabled={isPending} onClick={onClose} type="button" variant="ghost">
              {t("explorer:actions.cancel")}
            </Button>
            <Button disabled={isPending} type="submit">
              {t("explorer:actions.rename")}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

export function CreateEntryDialog({
  error,
  isPending,
  kind,
  onClose,
  onOpenChange,
  onSubmit,
  onValueChange,
  value,
}: {
  error: string | null;
  isPending: boolean;
  kind: NewEntryKind | null;
  onClose: () => void;
  onOpenChange: (open: boolean) => void;
  onSubmit: (event: FormEvent<HTMLFormElement>) => void;
  onValueChange: (value: string) => void;
  value: string;
}) {
  const { t } = useTranslation("explorer");
  const isFile = kind === "file";

  return (
    <Dialog onOpenChange={onOpenChange} open={kind !== null}>
      <DialogContent showCloseButton={!isPending}>
        <DialogHeader>
          <DialogTitle>
            {isFile
              ? t("explorer:newEntry.fileDialogTitle")
              : t("explorer:newEntry.folderDialogTitle")}
          </DialogTitle>
          <DialogDescription>
            {isFile
              ? t("explorer:newEntry.fileDescription")
              : t("explorer:newEntry.folderDescription")}
          </DialogDescription>
        </DialogHeader>
        <form className="flex flex-col gap-4" onSubmit={onSubmit}>
          <FieldGroup>
            <Field data-invalid={Boolean(error)}>
              <FieldLabel htmlFor="create-entry">{t("explorer:newEntry.nameLabel")}</FieldLabel>
              <Input
                aria-invalid={Boolean(error)}
                autoFocus
                disabled={isPending}
                id="create-entry"
                onChange={(event) => onValueChange(event.target.value)}
                onFocus={(event) => {
                  const input = event.currentTarget;
                  const dotIndex = isFile ? input.value.lastIndexOf(".") : -1;
                  input.setSelectionRange(0, dotIndex > 0 ? dotIndex : input.value.length);
                }}
                value={value}
              />
              <FieldError>{error}</FieldError>
            </Field>
          </FieldGroup>
          <DialogFooter>
            <Button disabled={isPending} onClick={onClose} type="button" variant="ghost">
              {t("explorer:actions.cancel")}
            </Button>
            <Button disabled={isPending} type="submit">
              {t("explorer:actions.create")}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

export function DeleteDialog({
  entries,
  isPending,
  onClose,
  onConfirm,
  onOpenChange,
}: {
  entries: DirectoryEntry[];
  isPending: boolean;
  onClose: () => void;
  onConfirm: () => void;
  onOpenChange: (open: boolean) => void;
}) {
  const { t } = useTranslation("explorer");
  const description =
    entries.length === 1
      ? t("explorer:deleteConfirm.single", { name: entries[0].name })
      : t("explorer:deleteConfirm.multiple", { number: entries.length });

  return (
    <Dialog onOpenChange={onOpenChange} open={entries.length > 0}>
      <DialogContent showCloseButton={!isPending}>
        <DialogHeader>
          <DialogTitle>{t("explorer:deleteConfirm.title")}</DialogTitle>
          <DialogDescription>
            {description}
            {t("explorer:deleteConfirm.hint")}
          </DialogDescription>
        </DialogHeader>
        <DialogFooter>
          <Button disabled={isPending} onClick={onClose} type="button" variant="ghost">
            {t("explorer:actions.cancel")}
          </Button>
          <Button disabled={isPending} onClick={onConfirm} type="button" variant="destructive">
            {t("explorer:actions.deletePermanent")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

export function ExplorerErrorAlert({ message, onRetry }: { message: string; onRetry: () => void }) {
  const { t } = useTranslation("explorer");

  return (
    <Alert variant="destructive">
      <TriangleAlert />
      <AlertTitle>{t("explorer:errors.unreadableLocation")}</AlertTitle>
      <AlertDescription>{message}</AlertDescription>
      <AlertAction>
        <Button onClick={onRetry} size="xs" type="button" variant="outline">
          {t("explorer:actions.retry")}
        </Button>
      </AlertAction>
    </Alert>
  );
}
