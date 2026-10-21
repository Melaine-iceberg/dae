import { useMemo } from "react";
import { useSetAtom } from "jotai";
import { useTranslation } from "react-i18next";
import { useQueries } from "@tanstack/react-query";
import { CheckCircleIcon, GitBranchIcon } from "@solar-icons/react/linear";

import {
  commands,
  type GitEntryStatus,
  type GitEntryStatusKind,
  type RecentItem,
} from "@/bindings";
import { Skeleton } from "@/components/ui/skeleton";
import { GIT_STATUS_PRESENTATION } from "@/features/explorer/git-status";
import { cn } from "@/lib/utils";

import { navigateToFolderAtom } from "./workspace-atoms";
import { SectionHeader, baseNameOf, formatRecentTime } from "./workspace-components";

/** Recent directories probed for a containing Git work tree. */
const DETECTION_LIMIT = 8;
/** Project cards shown on the overview. */
const PROJECTS_LIMIT = 4;
/** Repo status rescans are expensive; keep them warm for a minute. */
const STATS_STALE_MS = 60_000;

interface Project {
  root: string;
  name: string;
  branch: string;
  accessedAt: number;
}

/**
 * Developer-oriented landing section: the Git repositories the user worked in
 * most recently, with their branch and dirty state. Detection reuses the
 * explorer's `get_git_status` command — it runs on the backend's blocking
 * thread pool, and the worktree scan behind it is cached per worktree, so
 * probing every recent directory of one monorepo costs a single scan and the
 * first frame never waits on it.
 *
 * Two query phases, both keyed `["git-status", path]` so a recent directory
 * that IS a repo root costs a single scan:
 *  1. probe the most recent directories for a containing work tree (root +
 *     branch), dedupe by root in recency order;
 *  2. fetch root-level status for the dirty counts.
 */
export function ProjectsSection({ recents }: { recents: RecentItem[] | null }) {
  const { t } = useTranslation(["workspace", "explorer"]);
  const navigateToFolder = useSetAtom(navigateToFolderAtom);

  const recentDirs = useMemo(
    () => (recents ?? []).filter((item) => item.kind === "directory").slice(0, DETECTION_LIMIT),
    [recents],
  );

  const detections = useQueries({
    queries: recentDirs.map((item) => ({
      queryFn: () => commands.getGitStatus(item.path),
      queryKey: ["git-status", item.path],
      retry: false,
      staleTime: STATS_STALE_MS,
    })),
  });

  const projects = useMemo(() => {
    const byRoot = new Map<string, Project>();
    recentDirs.forEach((item, index) => {
      const data = detections[index]?.data;
      if (!data || byRoot.has(data.root)) return;
      byRoot.set(data.root, {
        root: data.root,
        name: baseNameOf(data.root),
        branch: data.branch,
        accessedAt: item.accessedAt,
      });
    });
    return [...byRoot.values()].slice(0, PROJECTS_LIMIT);
  }, [recentDirs, detections]);

  const stats = useQueries({
    queries: projects.map((project) => ({
      queryFn: () => commands.getGitStatus(project.root),
      queryKey: ["git-status", project.root],
      retry: false,
      staleTime: STATS_STALE_MS,
    })),
  });

  const detecting = detections.some((detection) => detection.isPending);

  // Skeleton only while there is something worth waiting for; the section
  // hides itself entirely when no recent directory is a Git work tree.
  if (recents === null || (projects.length === 0 && detecting && recentDirs.length > 0)) {
    return (
      <section aria-label={t("workspace:overview.projectsTitle")}>
        <SectionHeader title={t("workspace:overview.projectsTitle")} />
        <div className="flex flex-col gap-0.5">
          <Skeleton className="h-9 rounded-md" />
          <Skeleton className="h-9 rounded-md" />
        </div>
      </section>
    );
  }

  if (projects.length === 0) return null;

  return (
    <section aria-label={t("workspace:overview.projectsTitle")}>
      <SectionHeader title={t("workspace:overview.projectsTitle")} />
      <div className="flex flex-col gap-0.5">
        {projects.map((project, index) => (
          <ProjectRow
            entries={stats[index]?.data?.entries}
            key={project.root}
            onOpen={() => navigateToFolder(project.root)}
            project={project}
          />
        ))}
      </div>
    </section>
  );
}

function ProjectRow({
  entries,
  onOpen,
  project,
}: {
  entries: GitEntryStatus[] | undefined;
  onOpen: () => void;
  project: Project;
}) {
  const { t } = useTranslation(["workspace", "explorer"]);
  const counts = countByKind(entries);
  const dirty = counts !== undefined && Object.values(counts).some((count) => count > 0);

  return (
    <button
      className={cn(
        // The reference's row anatomy: a 36px line, the repository glyph in the
        // slot every other row uses, the branch as a chip beside the name, and
        // the dirty counts flush right in the status column's place. The card
        // this replaces asked four repositories to share two 90px blocks —
        // which made the least scannable section on the page the tallest one.
        "group flex h-9 w-full items-center gap-2.5 rounded-md px-2.5 text-left",
        "transition-colors duration-fast ease-standard hover:bg-accent focus-visible:ring-[3px] focus-visible:ring-ring/35 focus-visible:outline-none",
      )}
      onClick={onOpen}
      title={project.root}
      type="button"
    >
      <GitBranchIcon className="size-4 shrink-0 text-muted-foreground transition-colors group-hover:text-foreground" />
      <span className="min-w-0 max-w-[34%] shrink-0 truncate text-caption text-foreground">
        {project.name}
      </span>
      <span className="min-w-0 shrink truncate rounded-xs bg-secondary px-1.5 py-0.5 font-mono text-[0.625rem] leading-4 text-secondary-foreground">
        {project.branch}
      </span>
      <span className="ml-auto flex shrink-0 items-center gap-1.5">
        {counts !== undefined &&
          (dirty ? (
            (Object.keys(GIT_STATUS_PRESENTATION) as GitEntryStatusKind[]).map((kind) =>
              counts[kind] > 0 ? (
                <span
                  aria-label={t(`explorer:git.${kind}`)}
                  className={cn(
                    "shrink-0 rounded-xs px-1.5 font-mono text-[0.625rem] leading-4 font-semibold tabular-nums",
                    GIT_STATUS_PRESENTATION[kind].className,
                  )}
                  key={kind}
                  title={t(`explorer:git.${kind}`)}
                >
                  {GIT_STATUS_PRESENTATION[kind].letter} {counts[kind]}
                </span>
              ) : null,
            )
          ) : (
            <span className="flex shrink-0 items-center gap-1 text-micro text-muted-foreground">
              <CheckCircleIcon className="size-3.5 text-success" />
              {t("workspace:overview.projectClean")}
            </span>
          ))}
      </span>
      <span className="w-10 shrink-0 text-right font-mono text-[0.65625rem] text-meta tabular-nums">
        {formatRecentTime(project.accessedAt, t("workspace:recents.groups.yesterday"))}
      </span>
    </button>
  );
}

function countByKind(
  entries: GitEntryStatus[] | undefined,
): Record<GitEntryStatusKind, number> | undefined {
  if (entries === undefined) return undefined;

  const counts: Record<GitEntryStatusKind, number> = { modified: 0, added: 0, untracked: 0 };
  for (const entry of entries) {
    counts[entry.kind] += 1;
  }
  return counts;
}
