import { useEffect, useState, type FormEvent } from "react";
import { useTranslation } from "react-i18next";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { CheckIcon, CircleNotchIcon, CopyIcon } from "@phosphor-icons/react";

import { commands, type CloudProviderKind, type StoredCloudAccount } from "@/bindings";
import { translateBackendMessage } from "@/i18n/errors";

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
import { Label } from "@/components/ui/label";
import { cn } from "@/lib/utils";

import { DropboxIcon, GoogleDriveIcon, OneDriveIcon } from "./cloud-icons";

/** Must match the Rust loopback server; Dropbox requires it registered verbatim. */
const REDIRECT_URI = "http://127.0.0.1:51888";

const PROVIDERS = [
  { kind: "google_drive" as const, name: "Google Drive", Icon: GoogleDriveIcon },
  { kind: "onedrive" as const, name: "OneDrive", Icon: OneDriveIcon },
  { kind: "dropbox" as const, name: "Dropbox", Icon: DropboxIcon },
];

export function CloudAccountDialog({
  onAuthorized,
  onOpenChange,
  open,
}: {
  onAuthorized: (account: StoredCloudAccount) => void;
  onOpenChange: (open: boolean) => void;
  open: boolean;
}) {
  const { t } = useTranslation("sidebar");
  const [provider, setProvider] = useState<CloudProviderKind>("google_drive");
  const [clientId, setClientId] = useState("");
  const [clientSecret, setClientSecret] = useState("");
  const [authorizing, setAuthorizing] = useState(false);
  const [copied, setCopied] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (open) {
      setError(null);
      setCopied(false);
    }
  }, [open]);

  const submit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (clientId.trim() === "") {
      setError(t("cloud.clientIdRequired"));
      return;
    }

    setError(null);
    setAuthorizing(true);
    commands
      .authorizeCloudAccount({
        provider,
        clientId: clientId.trim(),
        clientSecret: clientSecret.trim() === "" ? null : clientSecret.trim(),
      })
      .then((account) => {
        setAuthorizing(false);
        onAuthorized(account);
        onOpenChange(false);
      })
      .catch((cause: unknown) => {
        setAuthorizing(false);
        setError(describeError(cause));
      });
  };

  const copyRedirectUri = () => {
    void writeText(REDIRECT_URI)
      .then(() => {
        setCopied(true);
        window.setTimeout(() => setCopied(false), 1500);
      })
      .catch((err: unknown) => console.warn("Unable to copy the redirect URI", err));
  };

  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>{t("cloud.dialogTitle")}</DialogTitle>
          <DialogDescription>{t("cloud.dialogDescription")}</DialogDescription>
        </DialogHeader>
        <form className="grid gap-4" onSubmit={submit}>
          <div className="grid gap-2">
            <Label>{t("cloud.provider")}</Label>
            <div className="grid grid-cols-3 gap-2">
              {PROVIDERS.map(({ Icon, kind, name }) => (
                <button
                  aria-pressed={provider === kind}
                  className={cn(
                    "flex flex-col items-center gap-1.5 rounded-md border px-2 py-2.5 transition-colors duration-fast ease-spring-fast hover:bg-accent/70",
                    provider === kind ? "border-primary bg-accent" : "border-border",
                  )}
                  disabled={authorizing}
                  key={kind}
                  onClick={() => {
                    setProvider(kind);
                    setError(null);
                  }}
                  type="button"
                >
                  <Icon className="size-6" />
                  <span className="text-xs">{name}</span>
                </button>
              ))}
            </div>
          </div>

          <div className="grid gap-2">
            <Label htmlFor="cloud-client-id">{t("cloud.clientId")}</Label>
            <Input
              autoComplete="off"
              disabled={authorizing}
              id="cloud-client-id"
              onChange={(event) => setClientId(event.target.value)}
              placeholder={t("cloud.clientIdPlaceholder")}
              value={clientId}
            />
          </div>

          <div className="grid gap-2">
            <Label htmlFor="cloud-client-secret">
              {provider === "onedrive" ? t("cloud.clientSecret") : t("cloud.clientSecretOptional")}
            </Label>
            <Input
              autoComplete="new-password"
              disabled={authorizing}
              id="cloud-client-secret"
              onChange={(event) => setClientSecret(event.target.value)}
              type="password"
              value={clientSecret}
            />
          </div>

          <div className="grid gap-2 rounded-md border border-border/60 bg-muted/40 p-3 text-xs">
            <p className="leading-relaxed text-muted-foreground">
              {t(`cloud.guide.${provider}`)}
            </p>
            <div className="flex items-center gap-2">
              <span className="shrink-0 text-muted-foreground">{t("cloud.redirectUri")}</span>
              <code className="min-w-0 flex-1 truncate rounded-xs bg-foreground/10 px-1.5 py-0.5 font-mono">
                {REDIRECT_URI}
              </code>
              <button
                aria-label={t("contextMenu.copyPath")}
                className="shrink-0 rounded-xs p-1 text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
                onClick={copyRedirectUri}
                type="button"
              >
                {copied ? (
                  <CheckIcon className="size-3.5 text-primary" />
                ) : (
                  <CopyIcon className="size-3.5" />
                )}
              </button>
            </div>
          </div>

          {error && <p className="text-[13px] text-destructive">{error}</p>}
          {authorizing && (
            <p className="text-xs leading-relaxed text-muted-foreground">{t("cloud.waitingHint")}</p>
          )}

          <DialogFooter>
            <Button disabled={authorizing} type="submit">
              {authorizing && <CircleNotchIcon className="animate-spin" />}
              {authorizing ? t("cloud.waiting") : t("cloud.authorize")}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

function describeError(error: unknown): string {
  let raw: string;
  if (typeof error === "object" && error !== null) {
    const message = (error as { message?: unknown }).message;
    raw = typeof message === "string" && message.trim() !== "" ? message : String(error);
  } else if (error instanceof Error && error.message) {
    raw = error.message;
  } else {
    raw = String(error);
  }

  const embedded = /message":\s*String\("(.+?)"\)/.exec(raw);
  return translateBackendMessage(embedded ? embedded[1] : raw);
}
