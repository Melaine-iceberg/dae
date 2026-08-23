import { useMemo } from "react";
import { useSetAtom } from "jotai";
import { useTranslation } from "react-i18next";
import { useQueries } from "@tanstack/react-query";
import { CheckCircleIcon, GitBranchIcon } from "@phosphor-icons/react";

import { commands, type GitEntryStatus, type GitEntryStatusKind, type RecentItem } from "@/bindings";
import { Skeleton } from "@/components/ui/skeleton";
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
 * thread pool and is cached per path, so the first frame never waits on it.
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
        <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
          <Skeleton className="h-[74px] rounded-2xl" />
          <Skeleton className="h-[74px] rounded-2xl" />
        </div>
      </section>
    );
  }

  if (projects.length === 0) return null;

  return (
    <section aria-label={t("workspace:overview.projectsTitle")}>
      <SectionHeader title={t("workspace:overview.projectsTitle")} />
      <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
        {projects.map((project, index) => (
          <ProjectCard
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

function ProjectCard({
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
        "group flex w-full flex-col gap-2 rounded-2xl bg-background p-3.5 text-left shadow-ambient-xs",
        "transition-[background-color,box-shadow,transform] duration-300 ease-spring-fast hover:-translate-y-0.5 hover:bg-accent/60 hover:shadow-ambient-sm",
        "focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none",
      )}
      onClick={onOpen}
      title={project.root}
      type="button"
    >
      <span className="flex w-full items-center gap-2.5">
        <span className="flex size-8 shrink-0 items-center justify-center rounded-lg bg-secondary transition-colors group-hover:bg-primary-container">
          <GitBranchIcon className="size-4 text-secondary-foreground transition-colors group-hover:text-on-primary-container" />
        </span>
        <span className="min-w-0 flex-1 truncate text-[13px] font-medium">{project.name}</span>
        <span className="shrink-0 text-xs text-muted-foreground tabular-nums">
          {formatRecentTime(project.accessedAt, t("workspace:recents.groups.yesterday"))}
        </span>
      </span>
      <span className="flex w-full items-center gap-1.5 pl-[42px]">
        <span className="max-w-[60%] truncate rounded-full bg-secondary px-2 py-0.5 font-mono text-[11px] leading-4 text-secondary-foreground">
          {project.branch}
        </span>
        {counts !== undefined &&
          (dirty ? (
            (Object.keys(BADGE_PRESENTATION) as GitEntryStatusKind[]).map((kind) =>
              counts[kind] > 0 ? (
                <span
                  aria-label={t(`explorer:git.${kind}`)}
                  className={cn(
                    "shrink-0 rounded-full px-1.5 text-[10px] leading-4 font-semibold tabular-nums",
                    BADGE_PRESENTATION[kind].className,
                  )}
                  key={kind}
                  title={t(`explorer:git.${kind}`)}
                >
                  {BADGE_PRESENTATION[kind].letter} {counts[kind]}
                </span>
              ) : null,
            )
          ) : (
            <span className="flex shrink-0 items-center gap-1 text-[11px] text-muted-foreground">
              <CheckCircleIcon className="size-3.5 text-emerald-600" />
              {t("workspace:overview.projectClean")}
            </span>
          ))}
      </span>
    </button>
  );
}

const BADGE_PRESENTATION: Record<GitEntryStatusKind, { className: string; letter: string }> = {
  modified: { className: "bg-amber-500/15 text-amber-600", letter: "M" },
  added: { className: "bg-emerald-500/15 text-emerald-600", letter: "A" },
  untracked: { className: "bg-sky-500/15 text-sky-600", letter: "U" },
};

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
