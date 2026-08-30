import { useCallback, useEffect, useState, type FormEvent } from "react";
import { useTranslation } from "react-i18next";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import {
  ArrowDownIcon,
  ArrowUpIcon,
  ArrowsClockwiseIcon,
  CheckIcon,
  CloudArrowDownIcon,
  DownloadSimpleIcon,
  GitBranchIcon,
  PlusIcon,
  UploadSimpleIcon,
  WarningIcon,
  XIcon,
} from "@phosphor-icons/react";

import { commands, events } from "@/bindings";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Field, FieldError, FieldGroup, FieldLabel } from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import { getFileOperationErrorMessage, translateBackendMessage } from "@/i18n/errors";
import { cn } from "@/lib/utils";

import { GIT_STATUS_QUERY_KEY } from "./git-status";

const GIT_BRANCHES_QUERY_KEY = "git-branches";

/**
 * 仓库分支信息。与 `useGitStatus` 一样按路径缓存，目录变更事件与
 * 窗口聚焦会触发重新拉取；所有 Git 操作完成后也会显式失效。
 */
function useGitBranches(root: string | null) {
  const queryClient = useQueryClient();

  useEffect(() => {
    const unlistenPromise = events.explorerDirectoryChanged.listen(() => {
      void queryClient.invalidateQueries({ queryKey: [GIT_BRANCHES_QUERY_KEY] });
    });
    return () => {
      void unlistenPromise.then((unlisten) => unlisten());
    };
  }, [queryClient]);

  const { data } = useQuery({
    enabled: root !== null,
    placeholderData: (previous) => previous,
    queryFn: () => commands.listGitBranches(root!),
    queryKey: [GIT_BRANCHES_QUERY_KEY, root],
    retry: false,
  });

  return data ?? null;
}

/**
 * 状态栏的 Git 区域：分支徽标（点击弹出分支菜单）+ 同步按钮。
 * 菜单支持查看/切换本地与远程分支、新建分支、获取、拉取、推送与同步；
 * 操作失败时在状态栏上方弹出错误提示。
 */
export function StatusBarGit({ root, branch }: { root: string | null; branch: string | null }) {
  const { t } = useTranslation("explorer");
  const queryClient = useQueryClient();
  const branches = useGitBranches(root);

  const [isPending, setIsPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [isCreateOpen, setIsCreateOpen] = useState(false);

  /** 执行一个 Git 操作；成功后失效分支/状态查询，失败时返回错误文案。 */
  const run = useCallback(
    async (
      operation: () => Promise<unknown>,
      options?: { silent?: boolean },
    ): Promise<{ error?: string; ok: boolean }> => {
      setIsPending(true);
      setError(null);
      try {
        await operation();
        await Promise.all([
          queryClient.invalidateQueries({ queryKey: [GIT_BRANCHES_QUERY_KEY] }),
          queryClient.invalidateQueries({ queryKey: [GIT_STATUS_QUERY_KEY] }),
        ]);
        return { ok: true };
      } catch (caught) {
        const message = getGitErrorMessage(caught);
        if (!options?.silent) setError(message);
        return { error: message, ok: false };
      } finally {
        setIsPending(false);
      }
    },
    [queryClient],
  );

  const checkout = useCallback(
    (name: string, isRemote: boolean) => {
      if (!root || isPending) return;
      void run(() => commands.gitCheckoutBranch(root, name, isRemote));
    },
    [isPending, root, run],
  );

  /** 远程操作（获取/拉取/推送/同步）的统一入口，仅在可用时执行。 */
  const networkOp = useCallback(
    (operation: () => Promise<unknown>) => {
      if (!root || isPending || (branches?.remotes.length ?? 0) === 0) return;
      void run(operation);
    },
    [branches?.remotes.length, isPending, root, run],
  );

  if (!root || !branch) return null;

  const hasRemote = (branches?.remotes.length ?? 0) > 0;
  const networkDisabled = !hasRemote || isPending;
  const ahead = branches?.ahead ?? 0;
  const behind = branches?.behind ?? 0;

  return (
    <div className="relative flex min-w-0 shrink-0 items-center gap-0.5">
      <DropdownMenu>
        <DropdownMenuTrigger
          className={cn(
            "flex min-w-0 items-center gap-1 rounded-xs px-1.5 py-0.5 transition-colors",
            "hover:bg-accent hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none",
          )}
          title={t("git.menuTitle", { branch })}
        >
          <GitBranchIcon className="size-3.5 shrink-0" />
          <span className="max-w-48 truncate font-mono text-[11px] leading-4">{branch}</span>
          {(ahead > 0 || behind > 0) && (
            <span className="flex shrink-0 items-center gap-0.5 tabular-nums">
              {ahead > 0 && (
                <span className="flex items-center gap-0.5 text-emerald-600">
                  <ArrowUpIcon className="size-3" />
                  {ahead}
                </span>
              )}
              {behind > 0 && (
                <span className="flex items-center gap-0.5 text-sky-600">
                  <ArrowDownIcon className="size-3" />
                  {behind}
                </span>
              )}
            </span>
          )}
        </DropdownMenuTrigger>
        <DropdownMenuContent align="start" className="w-64">
          <DropdownMenuLabel className="flex min-w-0 items-center gap-1.5">
            <GitBranchIcon className="size-3.5 shrink-0" />
            <span className="truncate font-mono">{branch}</span>
            {branches?.detached ? (
              <span className="ml-auto shrink-0 text-[11px] font-normal">
                {t("git.detachedHint")}
              </span>
            ) : !branches?.hasUpstream ? (
              <span className="ml-auto shrink-0 text-[11px] font-normal">
                {t("git.noUpstreamHint")}
              </span>
            ) : ahead > 0 || behind > 0 ? (
              <span className="ml-auto shrink-0 text-[11px] font-normal tabular-nums">
                {t("git.aheadBehind", { ahead, behind })}
              </span>
            ) : null}
          </DropdownMenuLabel>
          <DropdownMenuSeparator />

          {branches && branches.local.length > 0 && (
            <>
              <DropdownMenuLabel>{t("git.localBranches")}</DropdownMenuLabel>
              {branches.local.map((localBranch) => (
                <DropdownMenuItem
                  disabled={isPending}
                  key={localBranch.name}
                  onClick={() => checkout(localBranch.name, false)}
                  title={t("git.switchToBranch", { branch: localBranch.name })}
                >
                  <GitBranchIcon className="size-4" />
                  <span className="min-w-0 flex-1 truncate">{localBranch.name}</span>
                  {localBranch.isCurrent && (
                    <CheckIcon className="ml-auto size-3.5 shrink-0 text-primary" />
                  )}
                </DropdownMenuItem>
              ))}
            </>
          )}

          {branches && branches.remote.length > 0 && (
            <>
              <DropdownMenuSeparator />
              <DropdownMenuLabel>{t("git.remoteBranches")}</DropdownMenuLabel>
              {branches.remote.map((remoteBranch) => (
                <DropdownMenuItem
                  disabled={isPending}
                  key={remoteBranch.name}
                  onClick={() => checkout(remoteBranch.name, true)}
                  title={t("git.trackRemoteBranch", { branch: remoteBranch.name })}
                >
                  <CloudArrowDownIcon className="size-4" />
                  <span className="min-w-0 flex-1 truncate">{remoteBranch.name}</span>
                </DropdownMenuItem>
              ))}
            </>
          )}

          <DropdownMenuSeparator />
          <DropdownMenuItem
            disabled={isPending}
            onClick={() => setIsCreateOpen(true)}
            title={t("git.newBranchTitle")}
          >
            <PlusIcon className="size-4" />
            {t("git.newBranch")}
          </DropdownMenuItem>
          <DropdownMenuItem
            disabled={networkDisabled}
            onClick={() => networkOp(() => commands.gitFetch(root))}
            title={t("git.fetchTitle")}
          >
            <CloudArrowDownIcon className="size-4" />
            {t("git.fetch")}
          </DropdownMenuItem>
          <DropdownMenuItem
            disabled={networkDisabled}
            onClick={() => networkOp(() => commands.gitPull(root))}
            title={t("git.pullTitle")}
          >
            <DownloadSimpleIcon className="size-4" />
            {t("git.pull")}
          </DropdownMenuItem>
          <DropdownMenuItem
            disabled={networkDisabled}
            onClick={() => networkOp(() => commands.gitPush(root))}
            title={t("git.pushTitle")}
          >
            <UploadSimpleIcon className="size-4" />
            {t("git.push")}
          </DropdownMenuItem>
          <DropdownMenuItem
            disabled={networkDisabled}
            onClick={() => networkOp(() => commands.gitSync(root))}
            title={t("git.syncTitle")}
          >
            <ArrowsClockwiseIcon className="size-4" />
            {t("git.sync")}
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>

      <button
        aria-label={t("git.syncTitle")}
        className={cn(
          "flex size-5 items-center justify-center rounded-xs transition-colors",
          "hover:bg-accent hover:text-foreground disabled:pointer-events-none disabled:opacity-50",
        )}
        disabled={networkDisabled}
        onClick={() => networkOp(() => commands.gitSync(root))}
        title={t("git.syncTitle")}
        type="button"
      >
        <ArrowsClockwiseIcon className={cn("size-3.5", isPending && "animate-spin")} />
      </button>

      {error && (
        <div
          className="animate-in fade-in-0 absolute bottom-6 left-0 z-50 flex w-80 items-start gap-2 rounded-lg bg-popover p-3 text-xs text-popover-foreground shadow-ambient-lg ring-1 ring-border"
          role="alert"
        >
          <WarningIcon className="mt-0.5 size-4 shrink-0 text-destructive" />
          <div className="min-w-0 flex-1">
            <p className="font-medium">{t("git.operationFailed")}</p>
            <p className="mt-0.5 line-clamp-3 break-words text-muted-foreground" title={error}>
              {error}
            </p>
          </div>
          <button
            aria-label={t("git.dismissError")}
            className="shrink-0 rounded-xs p-0.5 transition-colors hover:bg-accent"
            onClick={() => setError(null)}
            type="button"
          >
            <XIcon className="size-3.5" />
          </button>
        </div>
      )}

      <CreateBranchDialog
        onClose={() => setIsCreateOpen(false)}
        onOpenChange={setIsCreateOpen}
        onSubmit={(name, checkoutAfter) =>
          run(() => commands.gitCreateBranch(root, name, checkoutAfter), { silent: true })
        }
        open={isCreateOpen}
      />
    </div>
  );
}

function CreateBranchDialog({
  onClose,
  onOpenChange,
  onSubmit,
  open,
}: {
  onClose: () => void;
  onOpenChange: (open: boolean) => void;
  /** 返回操作结果；失败时对话框内联展示错误文案。 */
  onSubmit: (name: string, checkout: boolean) => Promise<{ error?: string; ok: boolean }>;
  open: boolean;
}) {
  const { t } = useTranslation("explorer");
  const [name, setName] = useState("");
  const [checkout, setCheckout] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [isSubmitting, setIsSubmitting] = useState(false);

  // 每次打开时重置表单。
  useEffect(() => {
    if (open) {
      setName("");
      setCheckout(true);
      setError(null);
    }
  }, [open]);

  const submit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const trimmed = name.trim();
    if (!trimmed || isSubmitting) return;

    setIsSubmitting(true);
    setError(null);
    const result = await onSubmit(trimmed, checkout);
    setIsSubmitting(false);
    if (result.ok) {
      onClose();
    } else {
      setError(result.error ?? null);
    }
  };

  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <DialogContent showCloseButton={!isSubmitting}>
        <DialogHeader>
          <DialogTitle>{t("git.newBranchTitle")}</DialogTitle>
          <DialogDescription>{t("git.newBranchDescription")}</DialogDescription>
        </DialogHeader>
        <form className="flex flex-col gap-4" onSubmit={submit}>
          <FieldGroup>
            <Field data-invalid={Boolean(error)}>
              <FieldLabel htmlFor="git-branch-name">{t("git.branchNameLabel")}</FieldLabel>
              <Input
                aria-invalid={Boolean(error)}
                autoFocus
                disabled={isSubmitting}
                id="git-branch-name"
                onChange={(event) => {
                  setName(event.target.value);
                  setError(null);
                }}
                onFocus={(event) => event.currentTarget.select()}
                value={name}
              />
              <FieldError>{error}</FieldError>
            </Field>
          </FieldGroup>
          <label className="flex items-center gap-2 text-[13px] text-muted-foreground">
            <input
              checked={checkout}
              className="size-4 accent-[var(--primary)]"
              onChange={(event) => setCheckout(event.target.checked)}
              type="checkbox"
            />
            {t("git.switchAfterCreate")}
          </label>
          <DialogFooter>
            <Button disabled={isSubmitting} onClick={onClose} type="button" variant="outline">
              {t("actions.cancel")}
            </Button>
            <Button disabled={isSubmitting || !name.trim()} type="submit">
              {t("actions.create")}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

/**
 * Git 命令的错误信息：后端统一返回 `git.*` 错误码（可带 ": detail" 后缀），
 * 优先按错误码翻译；否则回退到通用文件操作错误文案。
 */
export function getGitErrorMessage(error: unknown): string {
  const payload =
    typeof error === "object" && error !== null ? (error as { message?: unknown }) : null;
  if (payload && typeof payload.message === "string" && payload.message.startsWith("git.")) {
    return translateBackendMessage(payload.message);
  }
  return getFileOperationErrorMessage(error);
}
