import { useEffect, useState, type FormEvent } from "react";

import { commands, type Protocol, type StoredConnection } from "@/bindings";

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

const PROTOCOL_DEFAULT_PORTS: Record<Protocol, string> = {
  smb: "445",
  sftp: "22",
  ftp: "21",
  webdav: "443",
};

const PROTOCOL_LABELS: Record<Protocol, string> = {
  smb: "SMB / Windows 共享",
  sftp: "SFTP（即将支持）",
  ftp: "FTP（即将支持）",
  webdav: "WebDAV（即将支持）",
};

const AVAILABLE_PROTOCOLS: Protocol[] = ["smb"];

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
      setTest({ status: "failed", message: hostInvalid ? "请输入服务器地址" : "端口必须是 1-65535" });
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
      setError("请输入服务器地址");
      return;
    }
    if (portInvalid) {
      setError("端口必须是 1-65535");
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
          <DialogTitle>连接网络存储</DialogTitle>
          <DialogDescription>连接 SMB 服务器（NAS、Windows 共享）。匿名连接失败时请填写账号。</DialogDescription>
        </DialogHeader>
        <form className="grid gap-4" onSubmit={submit}>
          <div className="grid gap-2">
            <Label htmlFor="connect-protocol">协议</Label>
            <select
              aria-invalid={false}
              className="h-9 rounded-md border border-input bg-transparent px-3 text-[13px] outline-none focus-visible:border-ring focus-visible:ring-2 focus-visible:ring-ring/40 disabled:opacity-50"
              id="connect-protocol"
              onChange={(event) => {
                setProtocol(event.target.value as Protocol);
                setTest({ status: "idle" });
              }}
              value={protocol}
            >
              {(Object.keys(PROTOCOL_LABELS) as Protocol[]).map((candidate) => (
                <option disabled={!AVAILABLE_PROTOCOLS.includes(candidate)} key={candidate} value={candidate}>
                  {PROTOCOL_LABELS[candidate]}
                </option>
              ))}
            </select>
          </div>

          <div className="grid grid-cols-[1fr_7rem] gap-3">
            <div className="grid gap-2">
              <Label htmlFor="connect-host">服务器地址</Label>
              <Input
                aria-invalid={hostInvalid}
                id="connect-host"
                onChange={(event) => {
                  setHost(event.target.value);
                  setTest({ status: "idle" });
                }}
                placeholder="例如 nas.local 或 192.168.1.10"
                value={host}
              />
            </div>
            <div className="grid gap-2">
              <Label htmlFor="connect-port">端口</Label>
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
              <Label htmlFor="connect-username">账号（可选）</Label>
              <Input
                autoComplete="off"
                id="connect-username"
                onChange={(event) => setUsername(event.target.value)}
                value={username}
              />
            </div>
            <div className="grid gap-2">
              <Label htmlFor="connect-password">密码（可选）</Label>
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
            记住密码（保存在系统钥匙串中）
          </label>

          {test.status === "ok" && <p className="text-[13px] text-primary">连接测试成功</p>}
          {test.status === "failed" && <p className="text-[13px] text-destructive">{test.message}</p>}
          {error && <p className="text-[13px] text-destructive">{error}</p>}

          <DialogFooter className="gap-2 sm:justify-between">
            <Button disabled={test.status === "testing"} onClick={runTest} type="button" variant="outline">
              {test.status === "testing" ? "测试中…" : "测试连接"}
            </Button>
            <Button disabled={saving} type="submit">
              {saving ? "保存中…" : "连接"}
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
  return embedded ? embedded[1] : raw;
}
