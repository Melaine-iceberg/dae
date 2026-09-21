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

export const GIT_STATUS_QUERY_KEY = "git-status";

/**
 * 目录变更事件合并到这个间隔再拉取。
 *
 * 后端每个变更推一次事件，而一次构建、一次下载、一次 checkout 会推出成百上千
 * 个——每个都拉一次就是「每个事件一次全工作区 git2 状态扫描」，跟列举抢同一块
 * 磁盘。与 explorer 自身的刷新（`DIRECTORY_REFRESH_DELAY_MS`）同样的做法，只是
 * 这里的单次代价高得多，所以窗口也宽得多。`placeholderData` 会在等待期间继续
 * 显示上一次的徽标，所以合并不会让徽标闪烁。
 */
const GIT_STATUS_REFRESH_DELAY_MS = 400;

/**
 * 当前目录的 Git 装饰信息。状态由 git2 在后端阻塞线程计算；目录变更事件与
 * 窗口聚焦会触发重新拉取（合并后），保证徽标始终新鲜且不阻塞 UI。
 */
export function useGitStatus(directoryPath: string | null): ExplorerGitStatus | null {
  const queryClient = useQueryClient();

  useEffect(() => {
    let refreshTimeout: number | undefined;

    const unlistenPromise = events.explorerDirectoryChanged.listen(() => {
      window.clearTimeout(refreshTimeout);
      refreshTimeout = window.setTimeout(() => {
        refreshTimeout = undefined;
        void queryClient.invalidateQueries({ queryKey: [GIT_STATUS_QUERY_KEY] });
      }, GIT_STATUS_REFRESH_DELAY_MS);
    });

    return () => {
      window.clearTimeout(refreshTimeout);
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

/**
 * The single definition of how a Git working-tree state is coloured and
 * labelled. Three surfaces render these (the file-list badge, the project
 * card's dirty counts, the status bar's ahead/behind chips) and each used to
 * carry its own hand-copied Tailwind ramp — two of them with no dark variant,
 * so the same state read three slightly different greens and washed out on the
 * dark canvas. Exporting one table keeps the state colours a property of the
 * design system (--success / --warning / --info) rather than of each call site.
 */
export const GIT_STATUS_PRESENTATION: Record<
  GitEntryStatusKind,
  { className: string; label: string; letter: string }
> = {
  added: {
    className: "bg-success/15 text-success",
    label: "git.added",
    letter: "A",
  },
  modified: {
    className: "bg-warning/15 text-warning",
    label: "git.modified",
    letter: "M",
  },
  untracked: {
    className: "bg-info/15 text-info",
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
        "shrink-0 rounded-xs px-1.5 text-nano leading-4 font-semibold",
        presentation.className,
      )}
      title={t("git.statusTitle", { status: label })}
    >
      {presentation.letter}
    </span>
  );
}
