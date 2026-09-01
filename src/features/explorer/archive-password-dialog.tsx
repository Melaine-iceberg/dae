import { useEffect, useState, type FormEvent } from "react";
import { useTranslation } from "react-i18next";

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

export type ArchivePasswordMode = "extract" | "compress";

/**
 * Prompts for an archive password. In `extract` mode it is shown after an
 * encrypted archive rejected a passwordless attempt; in `compress` mode the
 * user opts into protecting a new 7z archive (confirmed twice).
 */
export function ArchivePasswordDialog({
  archiveName,
  error,
  isPending,
  mode,
  onOpenChange,
  onSubmit,
  open,
}: {
  archiveName: string;
  error: string | null;
  isPending: boolean;
  mode: ArchivePasswordMode;
  onOpenChange: (open: boolean) => void;
  onSubmit: (password: string) => void;
  open: boolean;
}) {
  const { t } = useTranslation("explorer");
  const [password, setPassword] = useState("");
  const [confirmPassword, setConfirmPassword] = useState("");
  const [validationError, setValidationError] = useState<string | null>(null);

  useEffect(() => {
    if (open) {
      setPassword("");
      setConfirmPassword("");
      setValidationError(null);
    }
  }, [open]);

  const submit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();

    if (!password) {
      setValidationError(t("archivePassword.passwordRequired"));
      return;
    }
    if (mode === "compress" && password !== confirmPassword) {
      setValidationError(t("archivePassword.mismatch"));
      return;
    }

    setValidationError(null);
    onSubmit(password);
  };

  const fieldError = validationError ?? error;

  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <DialogContent showCloseButton={!isPending}>
        <DialogHeader>
          <DialogTitle>
            {mode === "extract"
              ? t("archivePassword.extractTitle")
              : t("archivePassword.compressTitle")}
          </DialogTitle>
          <DialogDescription>
            {mode === "extract"
              ? t("archivePassword.extractDescription", { name: archiveName })
              : t("archivePassword.compressDescription")}
          </DialogDescription>
        </DialogHeader>
        <form className="flex flex-col gap-4" onSubmit={submit}>
          <FieldGroup>
            <Field data-invalid={Boolean(fieldError)}>
              <FieldLabel htmlFor="archive-password">
                {t("archivePassword.passwordLabel")}
              </FieldLabel>
              <Input
                aria-invalid={Boolean(fieldError)}
                autoComplete="off"
                autoFocus
                disabled={isPending}
                id="archive-password"
                onChange={(event) => {
                  setPassword(event.target.value);
                  setValidationError(null);
                }}
                type="password"
                value={password}
              />
              <FieldError>{fieldError}</FieldError>
            </Field>
            {mode === "compress" && (
              <Field>
                <FieldLabel htmlFor="archive-password-confirm">
                  {t("archivePassword.confirmPasswordLabel")}
                </FieldLabel>
                <Input
                  autoComplete="off"
                  disabled={isPending}
                  id="archive-password-confirm"
                  onChange={(event) => {
                    setConfirmPassword(event.target.value);
                    setValidationError(null);
                  }}
                  type="password"
                  value={confirmPassword}
                />
              </Field>
            )}
          </FieldGroup>
          <DialogFooter>
            <Button
              disabled={isPending}
              onClick={() => onOpenChange(false)}
              type="button"
              variant="outline"
            >
              {t("explorer:actions.cancel")}
            </Button>
            <Button disabled={isPending} type="submit">
              {mode === "extract"
                ? t("archivePassword.extract")
                : t("archivePassword.compress")}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
