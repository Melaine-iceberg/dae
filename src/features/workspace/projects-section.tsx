import { useMemo } from "react";
import { useSetAtom } from "jotai";
import { useTranslation } from "react-i18next";
import { useQueries } from "@tanstack/react-query";
import { CircleCheck, GitBranch } from "lucide-react";

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
          <Skeleton className="h-project-card rounded-md" />
          <Skeleton className="h-project-card rounded-md" />
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
        // Same card anatomy as LocationCard: hairline box on the plane's own
        // rung, hover brightens the hairline. Height comes from the token its
        // skeleton uses.
        "group state-layer flex h-project-card w-full flex-col gap-2 rounded-md border border-border bg-card p-3 text-left",
        "transition-[background-color,border-color] duration-fast ease-standard",
        "hover:border-input",
        "focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none",
      )}
      onClick={onOpen}
      title={project.root}
      type="button"
    >
      <span className="flex w-full items-center gap-2.5">
        <span className="flex size-8 shrink-0 items-center justify-center rounded-sm bg-secondary">
          <GitBranch className="size-4 text-secondary-foreground" />
        </span>
        <span className="min-w-0 flex-1 truncate text-body font-medium">{project.name}</span>
        <span className="shrink-0 text-caption text-muted-foreground tabular-nums">
          {formatRecentTime(project.accessedAt, t("workspace:recents.groups.yesterday"))}
        </span>
      </span>
      {/* 42px is derived, not chosen: it is the row above's 32px avatar
          (`size-8`) plus its 10px gap (`gap-2.5`), so this line starts exactly
          where the project name does. `pl-10.5` is that sum on the 4px grid —
          change either the avatar or the gap and this number is wrong. */}
      <span className="flex w-full items-center gap-1.5 pl-10.5">
        <span className="max-w-[60%] truncate rounded-xs bg-secondary px-2 py-0.5 font-mono text-micro leading-4 text-secondary-foreground">
          {project.branch}
        </span>
        {counts !== undefined &&
          (dirty ? (
            (Object.keys(GIT_STATUS_PRESENTATION) as GitEntryStatusKind[]).map((kind) =>
              counts[kind] > 0 ? (
                <span
                  aria-label={t(`explorer:git.${kind}`)}
                  className={cn(
                    "shrink-0 rounded-xs px-1.5 text-nano leading-4 font-semibold tabular-nums",
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
              <CircleCheck className="size-3.5 text-success" />
              {t("workspace:overview.projectClean")}
            </span>
          ))}
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
