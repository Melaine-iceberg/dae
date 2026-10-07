import { useMemo } from "react";
import { useTranslation } from "react-i18next";
import { useQuery } from "@tanstack/react-query";

import { commands, type GitEntryStatusKind } from "@/bindings";
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
 *
 * 分支查询共用这个窗口（`git-branches.tsx`）：事件源完全相同，单次代价也是同量级的
 * git2 全仓库遍历（打开仓库、列所有引用、再算 ahead/behind 的提交图）。
 */
export const GIT_REFRESH_DELAY_MS = 400;

/**
 * 新鲜到这个时长为止。
 *
 * 目录变更事件用 `invalidateQueries` 强制重取，**不受这个值约束**，所以真实的
 * 变化依然立刻可见；这个值只挡下「因为窗口重新获得焦点」而重跑一次 git2 全仓
 * 遍历的情况 —— 实测中新窗口显示后会为同一份数据再走一次，而它刚刚取过。
 *
 * 5 秒覆盖了预热窗口从启动到被显示的距离（预热延时 3 秒），所以池里的窗口带着
 * 自己已经取好的数据出现，不会一露面就重算。
 */
export const GIT_STALE_TIME_MS = 5_000;

/**
 * 当前目录的 Git 装饰信息。状态由 git2 在后端阻塞线程计算；目录变更事件会触发
 * 重新拉取（合并后，见 `useGitRefreshOnDirectoryChange`），窗口聚焦也会，但只限于
 * 数据已经不新鲜时（`GIT_STALE_TIME_MS`），保证徽标新鲜且不阻塞 UI。
 */
export function useGitStatus(directoryPath: string | null): ExplorerGitStatus | null {
  const { data } = useQuery({
    enabled: directoryPath !== null,
    placeholderData: (previous) => previous,
    queryFn: () => commands.getGitStatus(directoryPath!),
    queryKey: [GIT_STATUS_QUERY_KEY, directoryPath],
    retry: false,
    staleTime: GIT_STALE_TIME_MS,
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
