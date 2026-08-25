import { useEffect, useState, type FormEvent } from "react";
import { useTranslation } from "react-i18next";

import { commands, type Protocol, type StoredConnection } from "@/bindings";
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
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { i18n } from "@/i18n";

const PROTOCOL_DEFAULT_PORTS: Record<Protocol, string> = {
  smb: "445",
  sftp: "22",
  ftp: "21",
  webdav: "443",
};

const PROTOCOL_LABELS: Record<Protocol, string> = {
  get smb() {
    return i18n.t("sidebar:protocols.smb");
  },
  get sftp() {
    return i18n.t("sidebar:protocols.sftp");
  },
  get ftp() {
    return i18n.t("sidebar:protocols.ftp");
  },
  get webdav() {
    return i18n.t("sidebar:protocols.webdav");
  },
};

const AVAILABLE_PROTOCOLS: Protocol[] = ["smb", "sftp"];

type TestState = { status: "idle" } | { status: "testing" } | { status: "ok" } | { status: "failed"; message: string };

export function ConnectDialog({
  onOpenChange,
  onSaved,
  open,
}: {
  onOpenChange: (open: boolean) => void;
  onSaved: (connection: StoredConnection) => void;
  open: boolean;
}) {
  const { t } = useTranslation("sidebar");
  const [protocol, setProtocol] = useState<Protocol>("smb");
  const [host, setHost] = useState("");
  const [port, setPort] = useState("");
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [remember, setRemember] = useState(true);
  const [test, setTest] = useState<TestState>({ status: "idle" });
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (open) {
      setTest({ status: "idle" });
      setError(null);
    }
  }, [open]);

  const parsedPort = port.trim() === "" ? null : Number.parseInt(port, 10);
  const portInvalid = parsedPort !== null && (!Number.isInteger(parsedPort) || parsedPort < 1 || parsedPort > 65535);
  const hostInvalid = host.trim() === "";

  const buildInput = () => ({
    protocol,
    host: host.trim(),
    port: parsedPort,
    username: username.trim() === "" ? null : username.trim(),
    password: password === "" ? null : password,
    rememberPassword: remember,
  });

  const runTest = () => {
    if (hostInvalid || portInvalid) {
      setTest({
        status: "failed",
        message: hostInvalid ? t("connect.hostRequired") : t("connect.portRange"),
      });
      return;
    }

    setTest({ status: "testing" });
    commands
      .testConnection(buildInput())
      .then(() => setTest({ status: "ok" }))
      .catch((cause: unknown) => setTest({ status: "failed", message: describeError(cause) }));
  };

  const submit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (hostInvalid) {
      setError(t("connect.hostRequired"));
      return;
    }
    if (portInvalid) {
      setError(t("connect.portRange"));
      return;
    }

    setError(null);
    setSaving(true);
    commands
      .saveConnection(buildInput())
      .then((connection) => {
        setSaving(false);
        onSaved(connection);
        onOpenChange(false);
      })
      .catch((cause: unknown) => {
        setSaving(false);
        setError(describeError(cause));
      });
  };

  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>{t("network.connectStorage")}</DialogTitle>
          <DialogDescription>{t("connect.description")}</DialogDescription>
        </DialogHeader>
        <form className="grid gap-4" onSubmit={submit}>
          <div className="grid gap-2">
            <Label htmlFor="connect-protocol">{t("connect.protocol")}</Label>
            <Select
              items={PROTOCOL_LABELS}
              onValueChange={(value) => {
                setProtocol(value as Protocol);
                setTest({ status: "idle" });
              }}
              value={protocol}
            >
              <SelectTrigger id="connect-protocol">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {(Object.keys(PROTOCOL_LABELS) as Protocol[]).map((candidate) => (
                  <SelectItem
                    disabled={!AVAILABLE_PROTOCOLS.includes(candidate)}
                    key={candidate}
                    value={candidate}
                  >
                    {PROTOCOL_LABELS[candidate]}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>

          <div className="grid grid-cols-[1fr_7rem] gap-3">
            <div className="grid gap-2">
              <Label htmlFor="connect-host">{t("connect.host")}</Label>
              <Input
                aria-invalid={hostInvalid}
                id="connect-host"
                onChange={(event) => {
                  setHost(event.target.value);
                  setTest({ status: "idle" });
                }}
                placeholder={t("connect.hostPlaceholder")}
                value={host}
              />
            </div>
            <div className="grid gap-2">
              <Label htmlFor="connect-port">{t("connect.port")}</Label>
              <Input
                aria-invalid={portInvalid}
                id="connect-port"
                inputMode="numeric"
                onChange={(event) => {
                  setPort(event.target.value);
                  setTest({ status: "idle" });
                }}
                placeholder={PROTOCOL_DEFAULT_PORTS[protocol]}
                value={port}
              />
            </div>
          </div>

          <div className="grid grid-cols-2 gap-3">
            <div className="grid gap-2">
              <Label htmlFor="connect-username">{t("connect.username")}</Label>
              <Input
                autoComplete="off"
                id="connect-username"
                onChange={(event) => setUsername(event.target.value)}
                value={username}
              />
            </div>
            <div className="grid gap-2">
              <Label htmlFor="connect-password">{t("connect.password")}</Label>
              <Input
                autoComplete="new-password"
                id="connect-password"
                onChange={(event) => setPassword(event.target.value)}
                type="password"
                value={password}
              />
            </div>
          </div>

          <label className="flex items-center gap-2 text-[13px] text-muted-foreground">
            <input
              checked={remember}
              className="size-4 accent-[var(--primary)]"
              onChange={(event) => setRemember(event.target.checked)}
              type="checkbox"
            />
            {t("connect.rememberPassword")}
          </label>

          {test.status === "ok" && (
            <p className="text-[13px] text-primary">{t("connect.testOk")}</p>
          )}
          {test.status === "failed" && <p className="text-[13px] text-destructive">{test.message}</p>}
          {error && <p className="text-[13px] text-destructive">{error}</p>}

          <DialogFooter className="gap-2 sm:justify-between">
            <Button disabled={test.status === "testing"} onClick={runTest} type="button" variant="outline">
              {test.status === "testing" ? t("connect.testing") : t("connect.test")}
            </Button>
            <Button disabled={saving} type="submit">
              {saving ? t("connect.saving") : t("connect.connect")}
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

  // Plain-browser IPC rejects with the Rust debug repr, e.g.
  // InvokeError(Object {"kind": String("io"), "message": String("...")}).
  const embedded = /message":\s*String\("(.+?)"\)/.exec(raw);
  return translateBackendMessage(embedded ? embedded[1] : raw);
}
