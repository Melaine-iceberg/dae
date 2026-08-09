import { Fragment } from "react";

import {
  Breadcrumb,
  BreadcrumbEllipsis,
  BreadcrumbItem,
  BreadcrumbLink,
  BreadcrumbList,
  BreadcrumbPage,
  BreadcrumbSeparator,
} from "@/components/ui/breadcrumb";

import type { Breadcrumb as BreadcrumbData } from "./types";

interface ExplorerBreadcrumbsProps {
  breadcrumbs: BreadcrumbData[];
  onNavigate: (breadcrumb: BreadcrumbData) => void;
}

type VisibleBreadcrumb = { type: "breadcrumb"; breadcrumb: BreadcrumbData } | { type: "ellipsis" };

const MAX_VISIBLE_BREADCRUMBS = 4;

export function ExplorerBreadcrumbs({ breadcrumbs, onNavigate }: ExplorerBreadcrumbsProps) {
  const visibleBreadcrumbs = collapseBreadcrumbs(breadcrumbs);
  const currentPath = breadcrumbs.at(-1)?.path;

  return (
    <Breadcrumb className="min-w-0">
      <BreadcrumbList className="flex-nowrap">
        {visibleBreadcrumbs.map((item, index) => {
          const key = item.type === "ellipsis" ? "collapsed-path" : item.breadcrumb.path;

          return (
            <Fragment key={key}>
              {index > 0 && <BreadcrumbSeparator />}
              <BreadcrumbItem className="min-w-0">
                {item.type === "ellipsis" ? (
                  <BreadcrumbEllipsis />
                ) : item.breadcrumb.path === currentPath ? (
                  <BreadcrumbPage className="max-w-64 truncate">
                    {item.breadcrumb.name}
                  </BreadcrumbPage>
                ) : (
                  <BreadcrumbLink
                    className="max-w-40 truncate"
                    render={<button type="button" onClick={() => onNavigate(item.breadcrumb)} />}
                  >
                    {item.breadcrumb.name}
                  </BreadcrumbLink>
                )}
              </BreadcrumbItem>
            </Fragment>
          );
        })}
      </BreadcrumbList>
    </Breadcrumb>
  );
}

function collapseBreadcrumbs(breadcrumbs: BreadcrumbData[]): VisibleBreadcrumb[] {
  if (breadcrumbs.length <= MAX_VISIBLE_BREADCRUMBS) {
    return breadcrumbs.map((breadcrumb) => ({
      type: "breadcrumb",
      breadcrumb,
    }));
  }

  return [
    { type: "breadcrumb", breadcrumb: breadcrumbs[0] },
    { type: "ellipsis" },
    ...breadcrumbs.slice(-2).map((breadcrumb): VisibleBreadcrumb => ({
      type: "breadcrumb",
      breadcrumb,
    })),
  ];
}
