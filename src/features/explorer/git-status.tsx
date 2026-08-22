import { useEffect, useMemo } from "react";
import { useTranslation } from "react-i18next";
import { useQuery, useQueryClient } from "@tanstack/react-query";

import { commands, events, type GitEntryStatusKind } from "@/bindings";
import { cn } from "@/lib/utils";

import type { DirectoryEntry } from "./types";

export interface ExplorerGitStatus {
  branch: string;
  root: string;
  statusByName: Map<string, GitEntryStatusKind>;
  directoryUntracked: boolean;
}

const GIT_STATUS_QUERY_KEY = "git-status";

/**
 * 当前目录的 Git 装饰信息。状态由 git2 在后端阻塞线程计算并按路径缓存；
 * 目录变更事件与窗口聚焦会触发重新拉取，保证徽标始终新鲜且不阻塞 UI。
 */
export function useGitStatus(directoryPath: string | null): ExplorerGitStatus | null {
  const queryClient = useQueryClient();

  useEffect(() => {
    const unlistenPromise = events.explorerDirectoryChanged.listen(() => {
      void queryClient.invalidateQueries({ queryKey: [GIT_STATUS_QUERY_KEY] });
    });
    return () => {
      void unlistenPromise.then((unlisten) => unlisten());
    };
  }, [queryClient]);

  const { data } = useQuery({
    enabled: directoryPath !== null,
    placeholderData: (previous) => previous,
    queryFn: () => commands.getGitStatus(directoryPath!),
    queryKey: [GIT_STATUS_QUERY_KEY, directoryPath],
    retry: false,
  });

  return useMemo(() => {
    if (!data) return null;

    return {
      branch: data.branch,
      root: data.root,
      statusByName: new Map(data.entries.map((entry) => [entry.name, entry.kind])),
      directoryUntracked: data.directoryUntracked,
    };
  }, [data]);
}

/**
 * 解析单个条目的 Git 徽标。徽标按当前目录直接子项的名称索引，因此
 * 带多级 `relativePath` 的搜索结果跳过；当前目录整体未跟踪时所有
 * 直接子项均标为未跟踪。
 */
export function getEntryGitStatus(
  gitStatus: ExplorerGitStatus | null | undefined,
  entry: DirectoryEntry,
): GitEntryStatusKind | undefined {
  if (!gitStatus) return undefined;
  if (entry.relativePath && /[\\/]/.test(entry.relativePath)) return undefined;

  const status = gitStatus.statusByName.get(entry.name);
  if (status) return status;
  if (gitStatus.directoryUntracked) return "untracked";

  return undefined;
}

const GIT_STATUS_PRESENTATION: Record<
  GitEntryStatusKind,
  { className: string; label: string; letter: string }
> = {
  added: {
    className: "bg-emerald-500/15 text-emerald-600",
    label: "git.added",
    letter: "A",
  },
  modified: {
    className: "bg-amber-500/15 text-amber-600",
    label: "git.modified",
    letter: "M",
  },
  untracked: {
    className: "bg-sky-500/15 text-sky-600",
    label: "git.untracked",
    letter: "U",
  },
};

export function GitStatusBadge({ kind }: { kind: GitEntryStatusKind }) {
  const { t } = useTranslation("explorer");
  const presentation = GIT_STATUS_PRESENTATION[kind];
  const label = t(presentation.label);

  return (
    <span
      aria-label={t("git.statusTitle", { status: label })}
      className={cn(
        "shrink-0 rounded-full px-1.5 text-[10px] leading-4 font-semibold",
        presentation.className,
      )}
      title={t("git.statusTitle", { status: label })}
    >
      {presentation.letter}
    </span>
  );
}
